//! The tap targets on screen: one small layered window per managed
//! window, put where `focal_core::tab` says. Tap → [`Note::Tap`]; drag
//! past the slop → [`Note::Drag`] as it moves and [`Note::Drop`] on
//! release; the adapter turns those into `Event::Promoted` and
//! `Event::MoveTo`. Tab or border is `tap_style` in the config.
//!
//! Written 2026-09-03, compiled and unit-tested, **never launched** —
//! the first live run is Ryan's, on a day he is not living in Fusion.
//!
//! Why a real window per target rather than a hit region on the planned
//! wallpaper layer: a window is the one thing Windows will reliably
//! hit-test and deliver mouse messages to on top of another process's
//! window. Three flags keep it out of the way of everything else here:
//!
//! - `WS_EX_NOACTIVATE` — clicking it never makes it the foreground
//!   window, so the app underneath keeps focus and the foreground hook
//!   stays quiet (`WM_MOUSEACTIVATE` says the same thing again);
//! - `WS_EX_TOOLWINDOW` — no taskbar button, and `win::is_manageable`
//!   rejects it on sight (it has no title either), so the rescan never
//!   offers our own tabs to the engine;
//! - **no `WS_EX_TOPMOST`** (2026-09-04, REQUESTS §4): a target sits
//!   immediately *below* the window it belongs to — `SetWindowPos(target,
//!   hwndInsertAfter = window)`, re-asserted whenever the foreground
//!   changes, on minimize/restore, and on every rescan tick — so the
//!   window covers the target's inner region and the visible part is the
//!   ring outside. A target can never be above a window that is above
//!   its own window; a fullscreen foreground hides them all (the adapter
//!   decides that); a minimized, hidden or off-monitor window hides its
//!   own. Until then targets were topmost, and stayed over a fullscreen
//!   YouTube. The drop ghost is the one topmost window left here: it
//!   exists only during a drag and the mouse goes straight through it.
//!
//! The border variant is the same window wearing a ring-shaped region
//! (`SetWindowRgn`): the interior is not part of the window at all, so
//! clicks there reach the app. (`HTTRANSPARENT` would not do — it only
//! passes clicks to windows of the *same thread*.) Every target hides
//! while the layout is frozen for an overlay, so a screen capture never
//! has a tab in it.
//!
//! The drag is **polled, not captured**. `SetCapture` from a window
//! that does not hold the foreground only delivers mouse messages while
//! the pointer is over that window (documented), and ours never holds
//! the foreground by design — so the button-down is the only message
//! we take from the tab, and the main loop asks for the live button and
//! pointer every tick ([`poll`]) until the button comes up.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use focal_core::engine::WinId;
use focal_core::geometry::Rect;
use focal_core::tab::TapTarget;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, FillRect, SetWindowRgn, HDC,
    RGN_DIFF,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos,
    GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA,
    HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, MA_NOACTIVATE, SM_SWAPBUTTON, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::adapter::Note;
use crate::win;

/// The one window class every target shares.
const CLASS: PCWSTR = w!("focal_desk_tab");
/// Amber — the accent of the retired mock — as a GDI `0x00BBGGRR`.
const TAB_COLOR: COLORREF = COLORREF(0x003CA2E6);
/// Bone white for the drop highlight.
const GHOST_COLOR: COLORREF = COLORREF(0x00E7F0F4);
/// Tab opacity, 0–255: visible, not loud.
const TAB_ALPHA: u8 = 110;
/// Drop-highlight opacity.
const GHOST_ALPHA: u8 = 70;
/// How far the pointer must travel before a press becomes a drag, in
/// pixels — about 0.2" on the 8K panel. A tap with a shaky hand stays
/// a tap.
pub const SLOP_PX: i32 = 28;

/// The press in progress on some tab, if any. One at a time: the mouse
/// has one button, and every tab lives on the main thread.
static PRESS: Mutex<Option<Press>> = Mutex::new(None);

/// A button press on a tab and what it has become so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    /// The managed window whose tab was pressed.
    pub win: WinId,
    /// Where the button went down, virtual-desktop pixels.
    pub start: (i32, i32),
    /// Set once the pointer has travelled past the slop; never unset,
    /// so a drag that wanders back over its start is still a drag.
    pub dragging: bool,
}

/// What a press turns into when the button comes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Down and up without travelling: promote the window.
    Tap(WinId),
    /// Dragged and released here: move the window to the slot under
    /// the pointer.
    Drop(WinId, (i32, i32)),
}

impl Press {
    /// A fresh press at `at`.
    pub fn new(win: WinId, at: (i32, i32)) -> Self {
        Self { win, start: at, dragging: false }
    }

    /// The pointer moved to `at`. Returns the position while dragging —
    /// past `slop` pixels from the start, and from then on — so the
    /// caller can show the drop target; `None` while it is still a tap.
    pub fn moved(&mut self, at: (i32, i32), slop: i32) -> Option<(i32, i32)> {
        if !self.dragging {
            let (dx, dy) = (at.0 - self.start.0, at.1 - self.start.1);
            if dx.abs() > slop || dy.abs() > slop {
                self.dragging = true;
            }
        }
        self.dragging.then_some(at)
    }

    /// The button came up at `at`.
    pub fn released(self, at: (i32, i32)) -> Gesture {
        if self.dragging {
            Gesture::Drop(self.win, at)
        } else {
            Gesture::Tap(self.win)
        }
    }
}

/// The press slot. A poisoned lock (a panic elsewhere on this thread)
/// must not disable every tab, so the poison is simply ignored.
fn lock() -> MutexGuard<'static, Option<Press>> {
    PRESS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The pointer, in virtual-desktop pixels.
fn cursor() -> (i32, i32) {
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    (pt.x, pt.y)
}

/// True while the primary mouse button is held. `WM_LBUTTONDOWN` means
/// the *logical* left button, so a swapped-buttons setup is followed
/// here, where the key state is physical.
fn button_down() -> bool {
    unsafe {
        let vk = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
        (GetAsyncKeyState(vk.0 as i32) as u16) & 0x8000 != 0
    }
}

/// Advance the press in progress from the live mouse state; called once
/// per tick by the main loop. Returns the note the movement amounts to:
/// `Drag` while the button is down past the slop, `Tap` or `Drop` when
/// it comes up, nothing otherwise.
pub fn poll() -> Option<Note> {
    let mut slot = lock();
    let press = slot.as_mut()?;
    let at = cursor();
    if button_down() {
        return press.moved(at, SLOP_PX).map(|(x, y)| Note::Drag(press.win, x, y));
    }
    let press = slot.take()?;
    Some(match press.released(at) {
        Gesture::Tap(win) => Note::Tap(win),
        Gesture::Drop(win, (x, y)) => Note::Drop(win, x, y),
    })
}

/// The managed window a target belongs to, stashed in its user data.
/// Zero is the ghost, which belongs to nobody.
fn owner_of(hwnd: HWND) -> WinId {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u64 }
}

/// Window procedure shared by every target. It records the press and
/// paints; [`poll`] does the rest from the main loop.
unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Never take focus: the app under the tab keeps it.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // Solid fill in the target's own color; the layered alpha set at
        // creation makes it translucent.
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut c_void);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let color = if owner_of(hwnd) == 0 { GHOST_COLOR } else { TAB_COLOR };
            let brush = CreateSolidBrush(color);
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
            LRESULT(1)
        }
        // The press starts here; every later step comes from `poll`. A
        // press on the ghost cannot happen (it is mouse-transparent), so
        // the owner is always a real window.
        WM_LBUTTONDOWN => {
            *lock() = Some(Press::new(owner_of(hwnd), cursor()));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Register the shared window class. A second registration fails
/// harmlessly, so this is safe to call more than once.
fn register_class() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(tab_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

/// Make one target window: a layered, non-activating popup with a
/// uniform alpha. `owner` goes in the user data so the procedure knows
/// whose tab was pressed. A target is never topmost (§4); the ghost is,
/// and is also `WS_EX_TRANSPARENT`: the mouse goes straight through it
/// while it marks the drop slot.
fn create(owner: WinId, alpha: u8, click_through: bool, topmost: bool) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let mut ex = WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        if click_through {
            ex |= WS_EX_TRANSPARENT;
        }
        if topmost {
            ex |= WS_EX_TOPMOST;
        }
        let hwnd = CreateWindowExW(
            ex,
            CLASS,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner as isize);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        Some(hwnd)
    }
}

/// Put a target window over its rectangle without activating anything:
/// immediately below `below` in z-order (or topmost, for the ghost),
/// shown or hidden as asked, and wearing the ring region if it is a
/// border (or losing a stale one if the style changed to tab). The
/// z-order part is skipped when the target already sits directly below
/// its window, so re-asserting it every tick costs one `GetWindow` and
/// never churns the desktop's reorder events.
fn place(hwnd: HWND, target: TapTarget, below: Option<HWND>, show: bool) {
    let b = target.bounds();
    let (x, y, w, h) = (
        b.x.round() as i32,
        b.y.round() as i32,
        b.w.round() as i32,
        b.h.round() as i32,
    );
    unsafe {
        let mut flags = SWP_NOACTIVATE | if show { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        let insert_after = match below {
            Some(owner) => {
                if win::next_below(owner) == Some(hwnd) {
                    flags |= SWP_NOZORDER;
                }
                owner
            }
            None => HWND_TOPMOST,
        };
        let _ = SetWindowPos(hwnd, Some(insert_after), x, y, w, h, flags);
        let region = match target {
            TapTarget::Tab(_) => None,
            TapTarget::Border { outer, inner } => {
                // Window coordinates: the whole window minus the hole
                // where the app shows through.
                let ring = CreateRectRgn(0, 0, w, h);
                let hole = CreateRectRgn(
                    (inner.x - outer.x).round() as i32,
                    (inner.y - outer.y).round() as i32,
                    (inner.right() - outer.x).round() as i32,
                    (inner.bottom() - outer.y).round() as i32,
                );
                CombineRgn(Some(ring), Some(ring), Some(hole), RGN_DIFF);
                let _ = DeleteObject(hole.into());
                Some(ring)
            }
        };
        // The system owns the region from here on; never delete it.
        SetWindowRgn(hwnd, region, true);
    }
}

/// Take a target off the screen without destroying it.
fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// True while a managed window can carry a visible target: it still
/// exists, is shown, is not minimized, and touches the managed monitor
/// (REQUESTS-2026-09-04 §4: minimized, hidden or off-monitor → no frame).
fn owner_on_screen(owner: HWND, monitor: Rect) -> bool {
    win::is_window(owner)
        && win::is_visible(owner)
        && !win::is_iconic(owner)
        && win::intersects(win::window_rect(owner), monitor)
}

/// Destroy a target window for good.
fn destroy(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// The on-screen targets: one window per managed window, plus the
/// ghost that marks the drop slot during a drag.
pub struct Tabs {
    /// Live target windows by the managed window they belong to.
    tabs: HashMap<WinId, HWND>,
    /// The drop highlight, created once and hidden between drags.
    ghost: Option<HWND>,
}

impl Tabs {
    /// Register the window class and create the (hidden) ghost.
    pub fn new() -> Self {
        register_class();
        Self {
            tabs: HashMap::new(),
            ghost: create(0, GHOST_ALPHA, true, true),
        }
    }

    /// Bring the target windows in line with `targets` — one per managed
    /// window, in virtual-desktop pixels — creating, moving, reshaping
    /// and destroying as needed, each directly below its own window in
    /// z-order. With `visible` false every target hides (laptop mode, an
    /// overlay holding the foreground, a fullscreen foreground); with it
    /// true a target still hides while its own window is minimized,
    /// hidden or off `monitor`.
    pub fn sync(&mut self, targets: &[(WinId, TapTarget)], visible: bool, monitor: Rect) {
        let wanted: HashSet<WinId> = targets.iter().map(|(id, _)| *id).collect();
        let gone: Vec<WinId> = self
            .tabs
            .keys()
            .filter(|id| !wanted.contains(id))
            .copied()
            .collect();
        for id in gone {
            if let Some(hwnd) = self.tabs.remove(&id) {
                destroy(hwnd);
            }
        }
        for &(id, target) in targets {
            let hwnd = match self.tabs.get(&id) {
                Some(&hwnd) => hwnd,
                None => match create(id, TAB_ALPHA, false, false) {
                    Some(hwnd) => {
                        self.tabs.insert(id, hwnd);
                        hwnd
                    }
                    None => continue,
                },
            };
            let owner = win::hwnd_of(id);
            let show = visible && owner_on_screen(owner, monitor);
            place(hwnd, target, Some(owner), show);
        }
    }

    /// How many target windows exist right now.
    pub fn count(&self) -> usize {
        self.tabs.len()
    }

    /// Show the drop highlight over `rect` (virtual-desktop pixels). The
    /// ghost is topmost so the slot lights up over whatever is there.
    pub fn show_ghost(&self, rect: Rect) {
        if let Some(ghost) = self.ghost {
            place(ghost, TapTarget::Tab(rect), None, true);
        }
    }

    /// Hide the drop highlight.
    pub fn hide_ghost(&self) {
        if let Some(ghost) = self.ghost {
            hide(ghost);
        }
    }

    /// Destroy every window we made. Windows would die with the process
    /// anyway; this is for a clean quit.
    pub fn clear(&mut self) {
        for (_, hwnd) in self.tabs.drain() {
            destroy(hwnd);
        }
        if let Some(ghost) = self.ghost.take() {
            destroy(ghost);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_press_is_a_tap() {
        let mut p = Press::new(7, (100, 100));
        assert_eq!(p.moved((110, 95), SLOP_PX), None, "inside the slop");
        assert_eq!(p.released((110, 95)), Gesture::Tap(7));
    }

    #[test]
    fn past_the_slop_it_is_a_drag_and_stays_one() {
        let mut p = Press::new(7, (100, 100));
        assert_eq!(p.moved((100 + SLOP_PX + 1, 100), SLOP_PX), Some((129, 100)));
        // Wandering back over the start does not turn it into a tap.
        assert_eq!(p.moved((100, 100), SLOP_PX), Some((100, 100)));
        assert_eq!(p.released((500, 600)), Gesture::Drop(7, (500, 600)));
    }

    #[test]
    fn the_slop_is_measured_on_either_axis() {
        let mut p = Press::new(1, (0, 0));
        assert_eq!(p.moved((0, SLOP_PX), SLOP_PX), None, "exactly the slop is not past it");
        assert_eq!(p.moved((0, SLOP_PX + 1), SLOP_PX), Some((0, SLOP_PX + 1)));
    }
}
