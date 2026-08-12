//! The service loop: turns Windows into `Event`s, turns `Command`s into
//! moving windows.
//!
//! Shape of a tick (~120 Hz):
//!
//! 1. pump the message queue (this is also how WinEvent hooks arrive);
//! 2. fold queued notifications — foreground changes, hotkeys, display
//!    changes — into engine events;
//! 3. rescan the window list a couple of times a second for opens/closes;
//! 4. promote whatever has held the foreground for `dwell_ms`;
//! 5. advance in-flight animations.
//!
//! Everything policy-shaped lives in `focal-core`; this file only knows
//! *how* to ask Windows, never *what* to do.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use focal_core::config::{Config, Screen};
use focal_core::engine::{Command, Engine, Event, WinId};
use focal_core::geometry::Rect;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_D, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, PeekMessageW,
    RegisterClassW, TranslateMessage, CW_USEDEFAULT, MSG, PM_REMOVE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DEVICECHANGE, WM_DISPLAYCHANGE, WM_HOTKEY, WNDCLASSW,
};

use crate::anim::{self, Flight};
use crate::dock;
use crate::frame;
use crate::win;

/// Notifications posted from callbacks into the main loop.
#[derive(Clone, Copy, Debug)]
pub enum Note {
    /// A window became the foreground window at this instant.
    Foreground(u64),
    /// The display topology changed; desk mode may have flipped.
    DisplaysChanged,
    /// A registered hotkey fired.
    Hotkey(i32),
}

/// Hotkey id: clear the focal stage (Ctrl+Alt+Space).
const HOTKEY_CLEAR: i32 = 1;
/// Hotkey id: force desk mode on or off (Ctrl+Alt+D), for testing away
/// from the 8K panel.
const HOTKEY_TOGGLE: i32 = 2;

/// The channel every callback posts into. Set once during [`run`].
static NOTES: OnceLock<Sender<Note>> = OnceLock::new();

/// Post a note to the main loop, ignoring failures (shutdown races).
pub fn post(note: Note) {
    if let Some(tx) = NOTES.get() {
        let _ = tx.send(note);
    }
}

/// Window procedure for our hidden window: the only messages we care
/// about are display/device changes and hotkeys.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DISPLAYCHANGE | WM_DEVICECHANGE => {
            post(Note::DisplaysChanged);
            LRESULT(0)
        }
        WM_HOTKEY => {
            post(Note::Hotkey(wparam.0 as i32));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Create the invisible top-level window that receives broadcasts.
/// (A message-only window would not get `WM_DISPLAYCHANGE`.)
fn create_hidden_window() -> windows::core::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = windows::core::w!("focal_desk_listener");
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            windows::core::w!("focal-desk"),
            WINDOW_STYLE(0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

/// Ask Windows for our two global hotkeys. Failure is non-fatal — the
/// service still works, you just lose the shortcut.
fn register_hotkeys(hwnd: HWND) {
    unsafe {
        let _ = RegisterHotKey(
            Some(hwnd),
            HOTKEY_CLEAR,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_SPACE.0 as u32,
        );
        let _ = RegisterHotKey(
            Some(hwnd),
            HOTKEY_TOGGLE,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_D.0 as u32,
        );
    }
}

/// Drain and dispatch pending window messages without blocking.
fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Runtime state of the service.
struct Service {
    engine: Engine,
    /// Origin of the managed monitor in virtual-desktop coordinates —
    /// the engine works in screen-local pixels, so we add this on.
    origin: (f32, f32),
    /// Where each window was last asked to go, in virtual-desktop
    /// pixels, so a repeat of the placement already in force can skip
    /// the OS call entirely.
    placed: HashMap<WinId, Rect>,
    /// Every window id the engine has been told about and not yet seen
    /// close — *including* ones it declined to manage. The engine keeps
    /// no record of a window it released, so without this the rescan
    /// would re-announce every unmanaged window on every pass.
    known: HashSet<WinId>,
    flights: Vec<Flight>,
    /// Window that currently holds the foreground, and since when.
    pending: Option<(u64, Instant)>,
    /// Set once the pending window has been promoted, so we only fire
    /// one event per activation.
    promoted_pending: bool,
    forced: bool,
}

impl Service {
    /// Build the service around an engine and the monitor it manages.
    fn new(cfg: Config) -> Self {
        let forced = cfg.force_active;
        Self {
            engine: Engine::new(cfg),
            origin: (0.0, 0.0),
            placed: HashMap::new(),
            known: HashSet::new(),
            flights: Vec::new(),
            pending: None,
            promoted_pending: false,
            forced,
        }
    }

    /// Feed an event to the engine and start flights for the commands
    /// that come back.
    fn dispatch(&mut self, event: Event) {
        let commands = self.engine.handle(event);
        self.apply(commands);
    }

    /// Execute engine commands: translate to virtual-desktop pixels,
    /// compensate for the invisible frame, and launch the animation.
    fn apply(&mut self, commands: Vec<Command>) {
        let now = Instant::now();
        for command in commands {
            match command {
                Command::Place { win, to, animate } => {
                    let hwnd = win::hwnd_of(win);
                    let target = Rect::new(to.x + self.origin.0, to.y + self.origin.1, to.w, to.h);
                    // Re-issuing the placement already in force is a
                    // no-op. `WM_DEVICECHANGE` fires for every USB
                    // arrival and each one replays the whole layout,
                    // which would otherwise re-snap all 13 windows.
                    if self.placed.get(&win) == Some(&target)
                        && !self.flights.iter().any(|f| f.id == win)
                    {
                        continue;
                    }
                    self.placed.insert(win, target);
                    self.flights.retain(|f| f.id != win);
                    let from = win::window_rect(hwnd);
                    let compensated = frame::compensate_rect(hwnd, target);
                    if animate {
                        self.flights.push(Flight {
                            id: win,
                            hwnd,
                            from,
                            to: compensated,
                            started: now,
                            duration: Duration::from_millis(260),
                        });
                    } else {
                        anim::place_now(hwnd, compensated);
                    }
                }
                Command::Release(win) => {
                    self.placed.remove(&win);
                    self.flights.retain(|f| f.id != win);
                }
            }
        }
    }

    /// Compare the live window list against what we manage, emitting
    /// `Opened` and `Closed` events. Polling (rather than more hooks)
    /// keeps the tricky cases — cloaked UWP shells, splash screens that
    /// become real windows — in one place.
    fn rescan(&mut self) {
        let live: Vec<HWND> = win::enum_windows();
        let live_ids: HashSet<u64> = live.iter().map(|&h| win::id_of(h)).collect();
        for hwnd in &live {
            let id = win::id_of(*hwnd);
            // `insert` reports whether the id was new, so each window is
            // announced exactly once however the engine answers.
            if self.known.insert(id) {
                self.dispatch(Event::Opened(id, win::window_meta(*hwnd)));
            }
        }
        let gone: Vec<WinId> = self.known.difference(&live_ids).copied().collect();
        for id in gone {
            self.dispatch(Event::Closed(id));
            self.known.remove(&id);
            self.placed.remove(&id);
        }
    }

    /// How many known windows the engine has actually given a home slot
    /// — the rest are open windows it declined (no free slot).
    fn managed_count(&self) -> usize {
        self.known
            .iter()
            .filter(|&&id| self.engine.home_of(id).is_some())
            .count()
    }

    /// Handle a foreground change: the desktop clears the stage, any
    /// other window starts (or restarts) the dwell timer.
    fn on_foreground(&mut self, id: u64) {
        let hwnd = win::hwnd_of(id);
        if win::is_desktop(hwnd) {
            self.pending = None;
            self.promoted_pending = false;
            self.dispatch(Event::ClearStage);
            return;
        }
        if self.engine.focused() == Some(id) {
            return;
        }
        self.pending = Some((id, Instant::now()));
        self.promoted_pending = false;
    }

    /// Promote the pending window once it has held focus long enough.
    fn check_dwell(&mut self) {
        let Some((id, since)) = self.pending else {
            return;
        };
        if self.promoted_pending {
            return;
        }
        let dwell = Duration::from_millis(self.engine.config().dwell_ms);
        if since.elapsed() < dwell {
            return;
        }
        // Confirm it is still the foreground window before acting.
        let current = win::id_of(unsafe { GetForegroundWindow() });
        if current != id {
            self.pending = None;
            return;
        }
        self.promoted_pending = true;
        self.dispatch(Event::Promoted(id));
    }

    /// Re-evaluate desk mode and the managed monitor's geometry.
    fn refresh_displays(&mut self) {
        let (monitor, is_desk) = dock::managed_monitor();
        self.origin = (monitor.x, monitor.y);
        let screen = Screen::from_px(
            monitor.w as u32,
            monitor.h as u32,
            self.engine.config().screen_diagonal_in,
        );
        let commands = self.engine.set_screen(screen);
        self.apply(commands);
        let active = is_desk || self.forced;
        if active != self.engine.is_active() {
            self.dispatch(Event::DeskMode(active));
            println!(
                "desk mode {} ({}x{})",
                if active { "on" } else { "off" },
                monitor.w as u32,
                monitor.h as u32
            );
        }
    }
}

/// Run the service until the process is killed.
pub fn run(cfg: Config) -> windows::core::Result<()> {
    let (tx, rx): (Sender<Note>, Receiver<Note>) = channel();
    let _ = NOTES.set(tx);

    win::enable_dpi_awareness();
    let hwnd = create_hidden_window()?;
    register_hotkeys(hwnd);
    let _hook = crate::hook::install();

    let mut service = Service::new(cfg);
    service.refresh_displays();
    // The rescan is what places whatever was already open: every window
    // it announces comes straight back as a `Place` when active.
    service.rescan();
    println!(
        "managing {} of {} windows (desk mode {})",
        service.managed_count(),
        service.known.len(),
        if service.engine.is_active() { "on" } else { "off" }
    );

    println!("focal-desk running — Ctrl+Alt+Space clears the stage, Ctrl+Alt+D toggles desk mode");

    let mut last_scan = Instant::now();
    loop {
        pump_messages();

        while let Ok(note) = rx.try_recv() {
            match note {
                Note::Foreground(id) => service.on_foreground(id),
                Note::DisplaysChanged => service.refresh_displays(),
                Note::Hotkey(HOTKEY_CLEAR) => service.dispatch(Event::ClearStage),
                Note::Hotkey(HOTKEY_TOGGLE) => {
                    service.forced = !service.forced;
                    service.refresh_displays();
                }
                Note::Hotkey(_) => {}
            }
        }

        service.check_dwell();

        if last_scan.elapsed() >= Duration::from_millis(400) {
            last_scan = Instant::now();
            if service.engine.is_active() {
                service.rescan();
            }
        }

        service.flights = anim::tick(std::mem::take(&mut service.flights), Instant::now());
        std::thread::sleep(Duration::from_millis(8));
    }
}
