//! Window flight: our own ease-out animation between two rectangles.
//!
//! Windows has no window-move animation API, so we drive it ourselves.
//! Since 2026-09-04 (REQUESTS §9) **the window itself is never resized
//! while it moves**: what flies is a DWM thumbnail of it
//! ([`crate::snapshot`]), and the window gets exactly one `SetWindowPos`
//! when the flight lands. Should DWM refuse a thumbnail, the window
//! flies itself with `SWP_NOSIZE` and is still resized only once, at the
//! end. Either way:
//!
//! - `SWP_NOACTIVATE | SWP_NOZORDER` — a flight must never steal focus
//!   or reshuffle z-order, or promotion would feed itself endlessly;
//! - the snapshot lingers for a short grace after landing, so the window
//!   can paint at its new size before its likeness goes.
//!
//! (Until 2026-09-04 every frame re-sized the real window through a
//! `DeferWindowPos` batch; Fusion and KiCad sometimes broke under it.)

use std::time::{Duration, Instant};

use focal_core::engine::WinId;
use focal_core::geometry::Rect;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

use crate::snapshot::Snapshot;

/// How long a landed snapshot lingers over the re-placed window: about
/// two frames at 60 Hz, enough for most apps to paint at the new size.
pub const LANDING_GRACE: Duration = Duration::from_millis(40);

/// One window in motion.
pub struct Flight {
    pub id: WinId,
    pub hwnd: HWND,
    /// Where the window is, as Windows reports it (invisible borders
    /// included); the one final `SetWindowPos` goes to `to` in the same
    /// terms.
    pub from: Rect,
    pub to: Rect,
    pub started: Instant,
    pub duration: Duration,
    /// The likeness that flies instead of the window. `None` means DWM
    /// refused a thumbnail and the window flies itself, size unchanged.
    snapshot: Option<Snapshot>,
    /// The visible-frame path the snapshot follows: the window's visible
    /// frame at the origin, and the visible frame it will have at the
    /// destination.
    visible: (Rect, Rect),
    /// When the window was put in place; the snapshot lingers
    /// [`LANDING_GRACE`] past this.
    landed: Option<Instant>,
}

/// Where a flight is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Still moving; the eased progress, 0 to 1.
    Flying(f32),
    /// The window is (or is about to be) in place; the snapshot lingers.
    Landing,
    /// Over: the snapshot may go.
    Done,
}

/// The phase of a flight at `now`, from when it started, how long it
/// lasts, and when (if) it landed. Pure, so it is tested without a
/// window.
pub fn phase(started: Instant, duration: Duration, landed: Option<Instant>, now: Instant) -> Phase {
    if let Some(landed) = landed {
        return if now.saturating_duration_since(landed) >= LANDING_GRACE {
            Phase::Done
        } else {
            Phase::Landing
        };
    }
    let elapsed = now.saturating_duration_since(started).as_secs_f32();
    let total = duration.as_secs_f32().max(0.001);
    let t = (elapsed / total).clamp(0.0, 1.0);
    if t >= 1.0 {
        Phase::Landing
    } else {
        Phase::Flying(ease_out(t))
    }
}

impl Flight {
    /// Start a flight from `from` to `to` (reported rects), grabbing the
    /// snapshot now, while the window still sits at the origin.
    /// `visible` is the same path in visible-frame terms, which is what
    /// the snapshot covers.
    pub fn new(
        id: WinId,
        hwnd: HWND,
        from: Rect,
        to: Rect,
        visible: (Rect, Rect),
        now: Instant,
        duration: Duration,
    ) -> Self {
        Self {
            id,
            hwnd,
            from,
            to,
            started: now,
            duration,
            snapshot: Snapshot::new(hwnd, visible.0),
            visible,
            landed: None,
        }
    }

    /// True once the window has been put in place (the snapshot may
    /// still be lingering).
    pub fn has_landed(&self) -> bool {
        self.landed.is_some()
    }

    /// End the flight now: the window goes to its destination in one
    /// move, and the snapshot, if any, goes with the value.
    pub fn settle(self) {
        if self.landed.is_none() {
            place_now(self.hwnd, self.to);
        }
    }
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

/// Move a window immediately, with no animation. This is a window's one
/// resize per placement.
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

/// Move a window without touching its size — the fallback flight when
/// there is no snapshot. No `WM_SIZE` reaches the app until it lands.
fn move_only(hwnd: HWND, to: Rect) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            to.x as i32,
            to.y as i32,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
        );
    }
}

/// Advance every flight one frame; returns those still alive — flying,
/// or landed and lingering. A flight lands with the one `place_now` of
/// its window and is dropped (snapshot destroyed) after the grace.
pub fn tick(flights: Vec<Flight>, now: Instant) -> Vec<Flight> {
    let mut live = Vec::with_capacity(flights.len());
    for mut flight in flights {
        match phase(flight.started, flight.duration, flight.landed, now) {
            Phase::Flying(p) => {
                match &flight.snapshot {
                    Some(s) => s.move_to(lerp_rect(flight.visible.0, flight.visible.1, p)),
                    None => move_only(flight.hwnd, lerp_rect(flight.from, flight.to, p)),
                }
                live.push(flight);
            }
            Phase::Landing if flight.landed.is_none() => {
                place_now(flight.hwnd, flight.to);
                flight.landed = Some(now);
                if flight.snapshot.is_some() {
                    live.push(flight);
                }
            }
            Phase::Landing => live.push(flight),
            Phase::Done => {}
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

    #[test]
    fn a_flight_flies_lands_lingers_and_ends() {
        let t0 = Instant::now();
        let d = Duration::from_millis(260);
        assert_eq!(phase(t0, d, None, t0), Phase::Flying(0.0));
        assert!(matches!(
            phase(t0, d, None, t0 + Duration::from_millis(130)),
            Phase::Flying(p) if p > 0.5
        ));
        // At the end of the path the window is placed — once.
        assert_eq!(phase(t0, d, None, t0 + d), Phase::Landing);
        // Landed: the snapshot lingers for the grace, then goes.
        let landed = t0 + d;
        assert_eq!(phase(t0, d, Some(landed), landed), Phase::Landing);
        assert_eq!(phase(t0, d, Some(landed), landed + Duration::from_millis(10)), Phase::Landing);
        assert_eq!(phase(t0, d, Some(landed), landed + LANDING_GRACE), Phase::Done);
    }
}
