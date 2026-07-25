//! KEY SNIPPET 2 — Windows lies about window geometry.
//!
//! `GetWindowRect` includes an invisible resize border (~7px per side
//! on Win10/11). Place windows by that rect and every gutter comes out
//! ragged — the visible frames sit ~7px inside where you put them.
//! `DWMWA_EXTENDED_FRAME_BOUNDS` reports the *visible* frame. Measure
//! the difference per window (it varies by app!) and compensate.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

/// How far the visible frame sits inside the reported window rect:
/// (left, top, right, bottom), all >= 0 for normal windows.
pub fn frame_insets(hwnd: HWND) -> (i32, i32, i32, i32) {
    unsafe {
        let mut win = RECT::default();
        let mut vis = RECT::default();
        let _ = GetWindowRect(hwnd, &mut win);
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut vis as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        (
            vis.left - win.left,
            vis.top - win.top,
            win.right - vis.right,
            win.bottom - vis.bottom,
        )
    }
}

/// The rect to pass to SetWindowPos so the VISIBLE frame lands exactly
/// on `target`: expand outward by the insets.
pub fn compensate(target: RECT, insets: (i32, i32, i32, i32)) -> RECT {
    RECT {
        left: target.left - insets.0,
        top: target.top - insets.1,
        right: target.right + insets.2,
        bottom: target.bottom + insets.3,
    }
}
