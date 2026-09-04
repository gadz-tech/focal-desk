//! The frames on screen (REQUESTS-2026-09-04 §3, §4): four small
//! per-pixel-alpha layered windows per managed window — one per band of
//! the frame `focal_core::tab` lays out — painted by `focal_core::paint`
//! and blitted with `UpdateLayeredWindow`. Tap → [`Note::Tap`]; drag past
//! the slop → [`Note::Drag`] as it moves and [`Note::Drop`] on release;
//! the adapter turns those into `Event::Promoted` and `Event::MoveTo`.
//!
//! Written 2026-09-03 as one amber tab per window, reshaped 2026-09-04
//! into the frame; compiled and unit-tested, **never launched** — the
//! first live run is Ryan's.
//!
//! Why real windows: a window is the one thing Windows will reliably
//! hit-test and deliver mouse messages to on top of another process's
//! window. Why four per frame rather than one: a layered window needs a
//! bitmap the size of its whole rectangle, and one frame-sized bitmap
//! per window would be a hundred megabytes of mostly-transparent hole
//! on the 8K panel; four bands are a few hundred kilobytes. The flags:
//!
//! - `WS_EX_LAYERED` with `UpdateLayeredWindow` — per-pixel alpha: the
//!   rounded corners and the inner fade are painted, and a fully
//!   transparent pixel is not part of the window, so the corners are
//!   click-through with no region to manage;
//! - `WS_EX_NOACTIVATE` — clicking it never makes it the foreground
//!   window, so the app underneath keeps focus and the foreground hook
//!   stays quiet (`WM_MOUSEACTIVATE` says the same thing again);
//! - `WS_EX_TOOLWINDOW` — no taskbar button, and `win::is_manageable`
//!   rejects it on sight (it has no title either), so the rescan never
//!   offers our own frames to the engine;
//! - **no `WS_EX_TOPMOST`** (§4): the four bands hang in a chain
//!   immediately *below* the window they belong to — `SetWindowPos(band,
//!   hwndInsertAfter = window)` for the first, each next one below the
//!   previous — re-asserted whenever the foreground changes, on
//!   minimize/restore, and on every rescan tick — so the window covers
//!   the frame's inner region and the ring outside is what shows. A
//!   frame can never be above a window that is above its own; a
//!   fullscreen foreground hides them all (the adapter decides that); a
//!   minimized, hidden or off-monitor window hides its own. The drop
//!   ghost is the one topmost window left here: it exists only during a
//!   drag and the mouse goes straight through it.
//!
//! Every frame hides while the layout is frozen for an overlay, so a
//! screen capture never has one in it. Pixels are repainted only when a
//! band's geometry or its active state changes.
//!
//! The drag is **polled, not captured**. `SetCapture` from a window
//! that does not hold the foreground only delivers mouse messages while
//! the pointer is over that window (documented), and ours never holds
//! the foreground by design — so the button-down is the only message
//! we take from the frame, and the main loop asks for the live button
//! and pointer every tick ([`poll`]) until the button comes up.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use focal_core::engine::WinId;
use focal_core::geometry::Rect;
use focal_core::paint::{self, Pixels};
use focal_core::tab::Frame;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC, DeleteObject, FillRect,
    GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos,
    GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow,
    GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, MA_NOACTIVATE, SM_SWAPBUTTON,
    SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, ULW_ALPHA,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::adapter::Note;
use crate::win;

/// The one window class every frame band and the ghost share.
const CLASS: PCWSTR = w!("focal_desk_tab");
/// Bone white for the drop highlight, as a GDI `0x00BBGGRR`.
const GHOST_COLOR: COLORREF = COLORREF(0x00E7F0F4);
/// Drop-highlight opacity.
const GHOST_ALPHA: u8 = 70;
/// Amber — the 2026-09-03 tab — as a GDI `0x00BBGGRR`: what a band falls
/// back to if Windows refuses the per-pixel update, so a frame is never
/// invisible.
const FALLBACK_COLOR: COLORREF = COLORREF(0x003CA2E6);
/// The fallback's uniform opacity.
const FALLBACK_ALPHA: u8 = 110;
/// How far the pointer must travel before a press becomes a drag, in
/// pixels — about 0.2" on the 8K panel. A tap with a shaky hand stays
/// a tap.
pub const SLOP_PX: i32 = 28;

/// The press in progress on some frame, if any. One at a time: the
/// mouse has one button, and every frame lives on the main thread.
static PRESS: Mutex<Option<Press>> = Mutex::new(None);

/// A button press on a frame and what it has become so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    /// The managed window whose frame was pressed.
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
/// must not disable every frame, so the poison is simply ignored.
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

/// The managed window a band belongs to, stashed in its user data.
/// Zero is the ghost, which belongs to nobody.
fn owner_of(hwnd: HWND) -> WinId {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u64 }
}

/// Window procedure shared by every band and the ghost. It records the
/// press and paints the ghost; [`poll`] does the rest from the main loop.
/// A band's pixels come from `UpdateLayeredWindow`, never from here.
unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Never take focus: the app under the frame keeps it.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // The ghost is a solid fill under a uniform alpha, and so is a
        // band that fell back from per-pixel painting; a band painted
        // through `UpdateLayeredWindow` never shows this fill.
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut c_void);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let color = if owner_of(hwnd) == 0 { GHOST_COLOR } else { FALLBACK_COLOR };
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

/// Make one window of the shared class: layered, non-activating, no
/// taskbar button, `owner` in the user data so the procedure knows whose
/// frame was pressed. A frame band is painted through
/// `UpdateLayeredWindow` and must never have `SetLayeredWindowAttributes`
/// called on it (Windows then refuses per-pixel updates); the ghost is
/// the other way round — a uniform alpha over a solid fill — and is also
/// click-through and topmost.
fn create(owner: WinId, ghost: bool) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let mut ex = WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        if ghost {
            ex |= WS_EX_TRANSPARENT | WS_EX_TOPMOST;
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
        if ghost {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), GHOST_ALPHA, LWA_ALPHA);
        }
        Some(hwnd)
    }
}

/// Blit `pixels` into a band window at `at` (virtual-desktop pixels):
/// the one call that both sizes the window and gives it its per-pixel
/// content. A 32-bit top-down DIB is filled from the buffer and handed
/// to `UpdateLayeredWindow` with per-pixel source alpha; the DIB is
/// released at once — Windows keeps its own copy. Returns whether
/// Windows took the update; an empty band counts as taken.
fn blit(hwnd: HWND, at: Rect, pixels: &Pixels) -> bool {
    if pixels.w == 0 || pixels.h == 0 {
        return true;
    }
    let mut taken = false;
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: pixels.w as i32,
                biHeight: -(pixels.h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            if !bits.is_null() {
                std::ptr::copy_nonoverlapping(
                    pixels.data.as_ptr(),
                    bits as *mut u32,
                    pixels.w * pixels.h,
                );
                let old = SelectObject(mem, bitmap.into());
                let dst = POINT { x: at.x.round() as i32, y: at.y.round() as i32 };
                let size = SIZE { cx: pixels.w as i32, cy: pixels.h as i32 };
                let src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                taken = UpdateLayeredWindow(
                    hwnd,
                    Some(screen),
                    Some(&dst as *const POINT),
                    Some(&size as *const SIZE),
                    Some(mem),
                    Some(&src as *const POINT),
                    COLORREF(0),
                    Some(&blend as *const BLENDFUNCTION),
                    ULW_ALPHA,
                )
                .is_ok();
                SelectObject(mem, old);
            }
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
    taken
}

/// Give up on per-pixel painting for a band: a uniform alpha over the
/// amber fill `tab_proc` draws, i.e. the 2026-09-03 tab. Once
/// `SetLayeredWindowAttributes` has been called the window cannot go
/// back to `UpdateLayeredWindow`, so this is one-way, and the band is
/// remembered as solid so it is never blitted again.
fn fall_back_to_solid(hwnd: HWND) {
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), FALLBACK_ALPHA, LWA_ALPHA);
    }
}

/// Put a window over `rect` without activating anything: immediately
/// below `below` in z-order (or topmost, for the ghost), shown or hidden
/// as asked. The z-order part is skipped when the window already sits
/// directly below its owner, so re-asserting it every tick costs one
/// `GetWindow` and never churns the desktop's reorder events.
fn place(hwnd: HWND, rect: Rect, below: Option<HWND>, show: bool) {
    let (x, y, w, h) = (
        rect.x.round() as i32,
        rect.y.round() as i32,
        rect.w.round() as i32,
        rect.h.round() as i32,
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
    }
}

/// Take a window off the screen without destroying it.
fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Destroy a window for good.
fn destroy(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// True while a managed window can carry a visible frame: it still
/// exists, is shown, is not minimized, and touches the managed monitor
/// (REQUESTS-2026-09-04 §4: minimized, hidden or off-monitor → no frame).
fn owner_on_screen(owner: HWND, monitor: Rect) -> bool {
    win::is_window(owner)
        && win::is_visible(owner)
        && !win::is_iconic(owner)
        && win::intersects(win::window_rect(owner), monitor)
}

/// The four band windows of one frame and what they last painted.
struct FrameWin {
    /// Top, bottom, left, right — the order `Frame::bands` uses.
    bands: [HWND; 4],
    /// The geometry and active state the pixels on screen came from;
    /// nothing is repainted while these hold.
    painted: Option<(Frame, bool)>,
    /// Bands Windows refused to paint per-pixel, now solid amber; they
    /// are positioned but never blitted again.
    solid: [bool; 4],
}

/// The on-screen frames: four band windows per managed window, plus the
/// ghost that marks the drop slot during a drag.
pub struct Tabs {
    /// Live frames by the managed window they belong to.
    frames: HashMap<WinId, FrameWin>,
    /// The drop highlight, created once and hidden between drags.
    ghost: Option<HWND>,
}

impl Tabs {
    /// Register the window class and create the (hidden) ghost.
    pub fn new() -> Self {
        register_class();
        Self { frames: HashMap::new(), ghost: create(0, true) }
    }

    /// Bring the frames in line with `frames` — one per managed window,
    /// in virtual-desktop pixels, with whether its window is the OS
    /// foreground — creating, repainting, moving, reordering and
    /// destroying as needed, every band directly below its own window.
    /// With `visible` false every frame hides (laptop mode, an overlay
    /// holding the foreground, a fullscreen foreground); with it true a
    /// frame still hides while its own window is minimized, hidden or
    /// off `monitor`.
    pub fn sync(&mut self, frames: &[(WinId, Frame, bool)], visible: bool, monitor: Rect) {
        let wanted: HashSet<WinId> = frames.iter().map(|(id, _, _)| *id).collect();
        let gone: Vec<WinId> =
            self.frames.keys().filter(|id| !wanted.contains(id)).copied().collect();
        for id in gone {
            if let Some(fw) = self.frames.remove(&id) {
                for hwnd in fw.bands {
                    destroy(hwnd);
                }
            }
        }
        for (id, frame, active) in frames {
            if !self.frames.contains_key(id) {
                let mut bands = [HWND::default(); 4];
                let mut ok = true;
                for band in bands.iter_mut() {
                    match create(*id, false) {
                        Some(hwnd) => *band = hwnd,
                        None => ok = false,
                    }
                }
                if !ok {
                    for hwnd in bands {
                        if !hwnd.is_invalid() {
                            destroy(hwnd);
                        }
                    }
                    continue;
                }
                self.frames.insert(*id, FrameWin { bands, painted: None, solid: [false; 4] });
            }
            let Some(fw) = self.frames.get_mut(id) else {
                continue;
            };
            let owner = win::hwnd_of(*id);
            let show = visible && owner_on_screen(owner, monitor);
            let repaint = fw
                .painted
                .as_ref()
                .map_or(true, |(f, a)| f != frame || *a != *active);
            let rects = frame.bands();
            for k in 0..4 {
                let hwnd = fw.bands[k];
                if repaint && !fw.solid[k] {
                    let pixels = paint::paint_band(frame, rects[k], *active);
                    if !blit(hwnd, rects[k], &pixels) {
                        fall_back_to_solid(hwnd);
                        fw.solid[k] = true;
                    }
                }
                // The bands hang in a chain under the owner — the first
                // directly below the window, each next one below the
                // previous — so "already in place" holds for all four
                // and a settled frame costs no z-order calls at all.
                let below = if k == 0 { owner } else { fw.bands[k - 1] };
                place(hwnd, rects[k], Some(below), show);
            }
            if repaint {
                fw.painted = Some((frame.clone(), *active));
            }
        }
    }

    /// How many frames exist right now.
    pub fn count(&self) -> usize {
        self.frames.len()
    }

    /// Show the drop highlight over `rect` (virtual-desktop pixels). The
    /// ghost is topmost so the slot lights up over whatever is there.
    pub fn show_ghost(&self, rect: Rect) {
        if let Some(ghost) = self.ghost {
            place(ghost, rect, None, true);
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
        for (_, fw) in self.frames.drain() {
            for hwnd in fw.bands {
                destroy(hwnd);
            }
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
