//! DWM frame compensation — the thing that makes gutters exact.
//!
//! `GetWindowRect` includes an invisible resize border (about 7px per
//! side on Win10/11, and it varies by app). Place windows by that rect
//! and every visible frame sits inside where you asked, so the gutters
//! come out ragged. `DWMWA_EXTENDED_FRAME_BOUNDS` reports the *visible*
//! frame; the difference between the two is what we add back.

use focal_core::geometry::Rect;
use windows::Win32::Foundation::HWND;

use crate::win;

/// How far the visible frame sits inside the reported window rect:
/// (left, top, right, bottom), normally >= 0.
pub fn frame_insets(hwnd: HWND) -> (f32, f32, f32, f32) {
    let outer = win::window_rect(hwnd);
    let visible = win::visible_rect(hwnd);
    (
        visible.x - outer.x,
        visible.y - outer.y,
        outer.right() - visible.right(),
        outer.bottom() - visible.bottom(),
    )
}

/// Expand a target rectangle by this window's insets, so that the
/// *visible* frame lands exactly on the target.
pub fn compensate_rect(hwnd: HWND, target: Rect) -> Rect {
    let (l, t, r, b) = frame_insets(hwnd);
    Rect::new(target.x - l, target.y - t, target.w + l + r, target.h + t + b)
}
