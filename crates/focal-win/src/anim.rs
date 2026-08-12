//! Window flight: our own ease-out animation between two rectangles.
//!
//! Windows has no window-move animation API, so we drive it ourselves.
//! Two rules keep it safe:
//! - `SWP_NOACTIVATE | SWP_NOZORDER` — a flight must never steal focus
//!   or reshuffle z-order, or promotion would feed itself endlessly;
//! - batch every window moving this tick through `DeferWindowPos`, so
//!   the compositor sees one transaction instead of a dozen.

use std::time::{Duration, Instant};

use focal_core::engine::WinId;
use focal_core::geometry::Rect;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, SetWindowPos, HDWP, SWP_NOACTIVATE,
    SWP_NOZORDER,
};

/// One window in motion.
pub struct Flight {
    pub id: WinId,
    pub hwnd: HWND,
    pub from: Rect,
    pub to: Rect,
    pub started: Instant,
    pub duration: Duration,
}

/// Cubic ease-out: quick departure, gentle landing.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Interpolate between two rectangles.
fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect::new(
        from.x + (to.x - from.x) * t,
        from.y + (to.y - from.y) * t,
        from.w + (to.w - from.w) * t,
        from.h + (to.h - from.h) * t,
    )
}

/// Move a window immediately, with no animation.
pub fn place_now(hwnd: HWND, to: Rect) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            to.x as i32,
            to.y as i32,
            to.w as i32,
            to.h as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

/// Advance every flight one frame; returns those still in motion.
pub fn tick(flights: Vec<Flight>, now: Instant) -> Vec<Flight> {
    if flights.is_empty() {
        return flights;
    }
    let mut live = Vec::with_capacity(flights.len());
    unsafe {
        let mut hdwp: HDWP = BeginDeferWindowPos(flights.len() as i32).unwrap_or_default();
        for flight in flights {
            let elapsed = now.saturating_duration_since(flight.started).as_secs_f32();
            let total = flight.duration.as_secs_f32().max(0.001);
            let t = (elapsed / total).clamp(0.0, 1.0);
            let r = lerp_rect(flight.from, flight.to, ease_out(t));
            if !hdwp.is_invalid() {
                hdwp = DeferWindowPos(
                    hdwp,
                    flight.hwnd,
                    None,
                    r.x as i32,
                    r.y as i32,
                    r.w as i32,
                    r.h as i32,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .unwrap_or(hdwp);
            } else {
                place_now(flight.hwnd, r);
            }
            if t < 1.0 {
                live.push(flight);
            }
        }
        if !hdwp.is_invalid() {
            let _ = EndDeferWindowPos(hdwp);
        }
    }
    live
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_is_bounded_and_monotonic() {
        assert!((ease_out(0.0) - 0.0).abs() < 1e-6);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-6);
        assert!(ease_out(0.25) > 0.25, "ease-out starts fast");
    }

    #[test]
    fn lerp_hits_both_ends() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(100.0, 50.0, 20.0, 30.0);
        assert_eq!(lerp_rect(a, b, 0.0), a);
        assert_eq!(lerp_rect(a, b, 1.0), b);
    }
}
