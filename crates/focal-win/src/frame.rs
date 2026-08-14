//! DWM frame compensation — the thing that makes gutters exact.
//!
//! `GetWindowRect` includes an invisible resize border (about 7px per
//! side on Win10/11, and it varies by app). Place windows by that rect
//! and every visible frame sits inside where you asked, so the gutters
//! come out ragged. `DWMWA_EXTENDED_FRAME_BOUNDS` reports the *visible*
//! frame; the difference between the two is what we add back.
//!
//! **Measure once, in a known-good state.** The insets are a property of
//! the window, not of where it happens to be: re-measuring while a
//! window is mid-flight, maximized, or freshly torn out of a browser
//! tab reads bounds DWM has not settled yet. Feeding that back into the
//! next placement is how a window drifts a few pixels per promotion
//! until its frame is visibly wrong — the Edge tear-out bug. The
//! adapter caches what this module returns and never re-measures.

use focal_core::geometry::Rect;
use windows::Win32::Foundation::HWND;

use crate::win;

/// Per-side distance from the reported window rect to the visible frame.
pub type Insets = (f32, f32, f32, f32);

/// No compensation — used when a measurement can't be trusted.
pub const NONE: Insets = (0.0, 0.0, 0.0, 0.0);

/// Largest inset we believe. Real borders are ~7px; anything beyond
/// this means we measured a window in a transient state, and applying
/// it would visibly mis-frame the window.
const MAX_INSET: f32 = 32.0;

/// Measure this window's frame insets, or return [`NONE`] if the
/// numbers look like a window that isn't settled.
///
/// Callers must only measure a restored, stationary window — see
/// [`crate::adapter`], which measures once at adoption and caches.
pub fn measure(hwnd: HWND) -> Insets {
    if win::is_maximized(hwnd) {
        return NONE;
    }
    let outer = win::window_rect(hwnd);
    let visible = win::visible_rect(hwnd);
    let insets = (
        visible.x - outer.x,
        visible.y - outer.y,
        outer.right() - visible.right(),
        outer.bottom() - visible.bottom(),
    );
    let sane = |v: f32| (0.0..=MAX_INSET).contains(&v);
    if sane(insets.0) && sane(insets.1) && sane(insets.2) && sane(insets.3) {
        insets
    } else {
        NONE
    }
}

/// Expand a target rectangle by known insets, so the *visible* frame
/// lands exactly on the target.
pub fn compensate(target: Rect, insets: Insets) -> Rect {
    let (l, t, r, b) = insets;
    Rect::new(target.x - l, target.y - t, target.w + l + r, target.h + t + b)
}
