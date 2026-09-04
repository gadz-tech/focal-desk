//! The flying snapshot (REQUESTS-2026-09-04 §9): **a window is never
//! resized while it moves.** A flight animates a DWM *thumbnail* of the
//! window — a live, scaled view the compositor draws, no copy on our
//! side — hosted in a small window of ours; the real window stays where
//! it is and gets exactly one `SetWindowPos` at the destination. Fusion
//! and KiCad re-create their GPU surfaces on every size change and
//! sometimes broke under the old per-frame resize: "screen grab low
//! res, scale that, refresh at new size" (Ryan, 12:20).
//!
//! The host is a plain (not layered) `WS_POPUP` — DWM thumbnails do not
//! render into layered windows — with `WS_EX_TOOLWINDOW` (no taskbar
//! button, and `win::is_manageable` rejects it), `WS_EX_NOACTIVATE`
//! (never takes the focus) and `WS_EX_TRANSPARENT` (the mouse goes
//! through it). It is `WS_EX_TOPMOST` for the quarter second it exists,
//! so the flight is seen over whatever it crosses, and it is destroyed a
//! couple of frames after the real window has been put in place, once
//! the window has had a chance to paint at its new size.
//!
//! The real window is left visible at its origin during the flight — not
//! hidden, not cloaked: either would make the rescan see it "closed". So
//! for ~260 ms there is the window and its flying likeness; the likeness
//! lands exactly over the destination, the window is resized under it,
//! and the likeness goes. If that reads badly live, the follow-up is
//! `DWMWA_CLOAK` for the flight with the rescan taught to skip windows in
//! flight. Compiled and unit-tested 2026-09-04, never launched.

use focal_core::geometry::Rect;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::CreateSolidBrush;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetWindowPos, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// The one window class every snapshot host shares.
const CLASS: PCWSTR = w!("focal_desk_snapshot");

/// A DWM thumbnail of a window, flying in a host window of ours.
pub struct Snapshot {
    /// Our host window, moved and scaled along the path.
    host: HWND,
    /// The DWM thumbnail registration (`HTHUMBNAIL`).
    thumb: isize,
}

/// Round a rectangle to the integer pixels `SetWindowPos` wants, never
/// smaller than one pixel a side.
fn px(r: Rect) -> (i32, i32, i32, i32) {
    (
        r.x.round() as i32,
        r.y.round() as i32,
        r.w.round().max(1.0) as i32,
        r.h.round().max(1.0) as i32,
    )
}

/// The host's window procedure: nothing but the default. (The crate's
/// `DefWindowProcW` is a Rust-ABI wrapper, so it cannot be the class
/// procedure directly.)
unsafe extern "system" fn host_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Register the host class: the default window procedure and a black
/// background (the thumbnail covers all of it; the brush shows only if
/// DWM lags a frame behind a resize). A second registration fails
/// harmlessly, so this is safe to call for every flight.
fn register_class() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hbrBackground: CreateSolidBrush(COLORREF(0)),
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

impl Snapshot {
    /// Register a thumbnail of `source` in a fresh host window over `at`
    /// (virtual-desktop pixels; the window's *visible* frame, since that
    /// is what the thumbnail shows). `None` when DWM refuses — no
    /// composition, a protected window — and the caller then flies the
    /// window itself, unresized, instead.
    pub fn new(source: HWND, at: Rect) -> Option<Self> {
        register_class();
        let (x, y, w, h) = px(at);
        unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let host = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOPMOST,
                CLASS,
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()?;
            let thumb = match DwmRegisterThumbnail(host, source) {
                Ok(thumb) => thumb,
                Err(_) => {
                    let _ = DestroyWindow(host);
                    return None;
                }
            };
            let snapshot = Self { host, thumb };
            snapshot.fit(w, h);
            let _ = SetWindowPos(
                host,
                Some(HWND_TOPMOST),
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            Some(snapshot)
        }
    }

    /// Point the thumbnail at the host's whole client area, `w` × `h`:
    /// the source scales to fill it, title bar included, fully opaque.
    fn fit(&self, w: i32, h: i32) {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT { left: 0, top: 0, right: w, bottom: h },
            rcSource: RECT::default(),
            opacity: 255,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
        };
        unsafe {
            let _ = DwmUpdateThumbnailProperties(self.thumb, &props);
        }
    }

    /// Move and scale the flying snapshot to `to` (virtual-desktop
    /// pixels). This is the per-frame call; the real window is not
    /// touched.
    pub fn move_to(&self, to: Rect) {
        let (x, y, w, h) = px(to);
        unsafe {
            let _ = SetWindowPos(self.host, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        self.fit(w, h);
    }
}

impl Drop for Snapshot {
    /// Unregister the thumbnail and destroy the host: nothing of the
    /// flight outlives the value.
    fn drop(&mut self) {
        unsafe {
            let _ = DwmUnregisterThumbnail(self.thumb);
            let _ = DestroyWindow(self.host);
        }
    }
}
