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
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use focal_core::config::{self, Config, Screen};
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
    WINDOW_STYLE, WM_DEVICECHANGE, WM_DISPLAYCHANGE, WM_HOTKEY, WM_RBUTTONUP, WNDCLASSW,
};

use crate::anim::{self, Flight};
use crate::dock;
use crate::frame;
use crate::log;
use crate::tray;
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
    /// The user picked something from the tray menu.
    Menu(u32),
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

/// Window procedure for our hidden window: display/device changes,
/// hotkeys, and the tray icon's callback all arrive here.
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
        // The tray icon reports the mouse message in lparam. The menu is
        // opened here, on the UI thread, because TrackPopupMenu runs its
        // own modal loop and must own the message pump while it does.
        tray::CALLBACK => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                post(Note::Menu(tray::show_menu(hwnd)));
            }
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

/// True when two rectangles match to within a pixel. The tolerance
/// absorbs the float-to-integer rounding that `SetWindowPos` forces, so
/// a window already in place is not nudged by half a pixel forever.
fn nearly(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 1.5
        && (a.y - b.y).abs() < 1.5
        && (a.w - b.w).abs() < 1.5
        && (a.h - b.h).abs() < 1.5
}

/// Runtime state of the service.
struct Service {
    engine: Engine,
    /// Origin of the managed monitor in virtual-desktop coordinates —
    /// the engine works in screen-local pixels, so we add this on.
    origin: (f32, f32),
    /// Every window id the engine has been told about and not yet seen
    /// close — *including* ones it declined to manage. The engine keeps
    /// no record of a window it released, so without this the rescan
    /// would re-announce every unmanaged window on every pass.
    known: HashSet<WinId>,
    /// Frame insets per window, measured once when the window is first
    /// placed and never again. Re-measuring a window that is mid-flight
    /// or maximized reads bounds DWM has not settled, and feeding that
    /// back compounds a few pixels every promotion until the frame is
    /// visibly wrong.
    insets: HashMap<WinId, frame::Insets>,
    flights: Vec<Flight>,
    /// Window that currently holds the foreground, and since when.
    pending: Option<(u64, Instant)>,
    /// Set once the pending window has been promoted, so we only fire
    /// one event per activation.
    promoted_pending: bool,
    forced: bool,
    /// The config file we watch for live edits.
    config_path: PathBuf,
    /// Its modification time as of the last successful read, so an
    /// untouched file costs one `stat` per tick and nothing more.
    config_seen: Option<SystemTime>,
    /// Our hidden window, which owns the tray icon.
    hwnd: HWND,
    /// The tooltip currently showing, so we only talk to the shell when
    /// the text actually changes.
    tip: String,
}

impl Service {
    /// Build the service around an engine, the monitor it manages, and
    /// the config file it watches.
    fn new(cfg: Config, config_path: PathBuf, hwnd: HWND) -> Self {
        let forced = cfg.force_active;
        // Record the file's current stamp so the first tick doesn't read
        // the config we were just handed back off disk.
        let config_seen = std::fs::metadata(&config_path)
            .and_then(|m| m.modified())
            .ok();
        Self {
            engine: Engine::new(cfg),
            origin: (0.0, 0.0),
            known: HashSet::new(),
            insets: HashMap::new(),
            flights: Vec::new(),
            pending: None,
            promoted_pending: false,
            forced,
            config_path,
            config_seen,
            hwnd,
            tip: String::new(),
        }
    }

    /// Hover text for the tray icon: the two things worth knowing at a
    /// glance — which mode it is in, and how much it is managing.
    fn tray_tip(&self) -> String {
        format!(
            "focal-desk — desk mode {} · {} of {} windows",
            if self.engine.is_active() { "on" } else { "off" },
            self.managed_count(),
            self.known.len()
        )
    }

    /// Push the tooltip to the shell, but only when it actually changed.
    /// This runs on every poll, so the comparison is the point.
    fn update_tray(&mut self) {
        let tip = self.tray_tip();
        if self.tip != tip {
            tray::set_tip(self.hwnd, &tip);
            self.tip = tip;
        }
    }

    /// Re-read the config file when it changes on disk, so tuning the
    /// gutter is a save rather than a restart.
    ///
    /// A bad edit keeps the configuration already running. The previous
    /// behavior — falling back to `Config::default()` — silently threw
    /// away every `[app]` rule over a single typo.
    fn check_config(&mut self) {
        let Ok(stamp) = std::fs::metadata(&self.config_path).and_then(|m| m.modified()) else {
            return;
        };
        if self.config_seen == Some(stamp) {
            return;
        }
        self.config_seen = Some(stamp);
        let Ok(text) = std::fs::read_to_string(&self.config_path) else {
            return;
        };
        match config::parse(&text) {
            Ok(mut cfg) => {
                // The file says how big the panel is; the panel says how
                // many pixels it has. The gutter needs both, so re-derive
                // the screen rather than trusting the parsed default.
                let (monitor, _) = dock::managed_monitor();
                cfg.screen =
                    Screen::from_px(monitor.w as u32, monitor.h as u32, cfg.screen_diagonal_in);
                self.dispatch(Event::Reconfigured(cfg));
                log::line("config reloaded");
            }
            Err(err) => log::line(&format!("config error, keeping previous config: {err}")),
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
                    // A maximized window keeps drawing its frame for the
                    // screen edge it thinks it is against, so restore it
                    // before we own its geometry — and re-measure, since
                    // the numbers we could have taken while it was
                    // maximized were meaningless.
                    if win::restore_window(hwnd) {
                        self.insets.remove(&win);
                    }
                    let insets = *self
                        .insets
                        .entry(win)
                        .or_insert_with(|| frame::measure(hwnd));
                    let from = win::window_rect(hwnd);
                    let compensated = frame::compensate(target, insets);
                    // Already there? Then there is nothing to do.
                    // `WM_DEVICECHANGE` fires on every USB arrival and
                    // replays the whole layout, which would otherwise
                    // re-snap all 13 windows each time.
                    //
                    // This asks where the window *is*, not where it was
                    // last sent. Those differ once the user drags one,
                    // and comparing the live rect is what lets a
                    // layout-wide re-assert put a dragged window back.
                    if !self.flights.iter().any(|f| f.id == win) && nearly(from, compensated) {
                        continue;
                    }
                    self.flights.retain(|f| f.id != win);
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
                    self.flights.retain(|f| f.id != win);
                    self.insets.remove(&win);
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
            self.insets.remove(&id);
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

    /// Finish every flight instantly, leaving each window at its final
    /// rectangle. Used when an overlay appears: a capture must see a
    /// still screen, not one mid-animation.
    fn settle_flights(&mut self) {
        for flight in std::mem::take(&mut self.flights) {
            anim::place_now(flight.hwnd, flight.to);
        }
    }

    /// Handle a foreground change: an ignored overlay freezes the
    /// layout, the desktop clears the stage, anything else starts (or
    /// restarts) the dwell timer.
    fn on_foreground(&mut self, id: u64) {
        let hwnd = win::hwnd_of(id);
        // Screen capture is the case that matters: Snipping Tool covers
        // the screen and any window that slides underneath lands in the
        // shot. Freeze first, and stop mid-flight windows where they
        // are rather than letting them keep moving.
        if self.engine.config().is_ignored(&win::window_meta(hwnd)) {
            self.pending = None;
            self.promoted_pending = false;
            if !self.engine.is_suspended() {
                self.settle_flights();
                self.dispatch(Event::Suspend(true));
                log::line("suspended for overlay");
            }
            return;
        }
        if self.engine.is_suspended() {
            self.dispatch(Event::Suspend(false));
            log::line("resumed");
        }
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
            log::line(&format!(
                "desk mode {} ({}x{})",
                if active { "on" } else { "off" },
                monitor.w as u32,
                monitor.h as u32
            ));
            self.update_tray();
        }
    }
}

/// Run the service until the process is killed.
pub fn run(cfg: Config, config_path: PathBuf) -> windows::core::Result<()> {
    // The logon task and a manual launch would otherwise put two services
    // on one desktop, each undoing the other's placements.
    if win::already_running() {
        log::line("another focal-desk is already running — this one is exiting");
        return Ok(());
    }

    let (tx, rx): (Sender<Note>, Receiver<Note>) = channel();
    let _ = NOTES.set(tx);

    win::enable_dpi_awareness();
    let hwnd = create_hidden_window()?;
    register_hotkeys(hwnd);
    let _hook = crate::hook::install();

    let mut service = Service::new(cfg, config_path, hwnd);
    if tray::add(hwnd, "focal-desk — starting") {
        log::line("tray icon added (Windows 11 files new icons under the taskbar's ^ overflow)");
    } else {
        log::line("tray icon REFUSED by the shell — the service is running but invisible");
    }
    service.refresh_displays();
    // The rescan is what places whatever was already open: every window
    // it announces comes straight back as a `Place` when active.
    service.rescan();
    service.update_tray();
    log::line(&format!(
        "managing {} of {} windows (desk mode {})",
        service.managed_count(),
        service.known.len(),
        if service.engine.is_active() { "on" } else { "off" }
    ));
    log::line(&format!(
        "elevated: {} — {}",
        if win::is_elevated() { "yes" } else { "no" },
        if win::is_elevated() {
            "windows of elevated apps move too"
        } else {
            "windows of elevated apps (an admin terminal) will not move"
        }
    ));
    log::line("running — Ctrl+Alt+Space clears the stage, Ctrl+Alt+D forces desk mode");

    let mut running = true;
    let mut last_scan = Instant::now();
    while running {
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
                Note::Menu(tray::CMD_CLEAR) => service.dispatch(Event::ClearStage),
                Note::Menu(tray::CMD_LOG) => tray::open_log(),
                Note::Menu(tray::CMD_QUIT) => running = false,
                Note::Menu(_) => {}
            }
        }

        service.check_dwell();

        if last_scan.elapsed() >= Duration::from_millis(400) {
            last_scan = Instant::now();
            service.check_config();
            if service.engine.is_active() {
                service.rescan();
            }
            service.update_tray();
        }

        service.flights = anim::tick(std::mem::take(&mut service.flights), Instant::now());
        std::thread::sleep(Duration::from_millis(8));
    }

    // Leave nothing behind: an icon that outlives its process sits in the
    // tray until the user happens to hover it.
    tray::remove(hwnd);
    log::line("quit — windows stay where they are");
    Ok(())
}
