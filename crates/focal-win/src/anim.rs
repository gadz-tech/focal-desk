//! KEY SNIPPET 3 — the flight.
//!
//! Windows has no window-move animation API; we run our own ~60 Hz
//! loop. Two rules keep it clean:
//! - `SWP_NOACTIVATE | SWP_NOZORDER` — the flight must never steal
//!   focus or reshuffle z-order, or promotion loops feed themselves;
//! - batch all windows moving in the same tick through
//!   `Begin/DeferWindowPos/EndDeferWindowPos` so the compositor treats
//!   it as one transaction (no interleaved half-states).

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
};

/// Cubic ease-out: fast start, gentle landing.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub struct Flight {
    pub hwnd: HWND,
    pub from: RECT,
    pub to: RECT,
    pub started_ms: u64,
    pub duration_ms: u64,
}

/// Integer interpolation with rounding.
fn lerp(a: i32, b: i32, t: f32) -> i32 {
    a + ((b - a) as f32 * t).round() as i32
}

/// One animation tick: advance every active flight one frame.
/// Returns the flights still in progress.
pub fn tick(flights: Vec<Flight>, now_ms: u64) -> Vec<Flight> {
    let mut live = Vec::new();
    unsafe {
        let mut hdwp = BeginDeferWindowPos(flights.len() as i32).unwrap_or_default();
        for f in flights {
            let t = ((now_ms - f.started_ms) as f32 / f.duration_ms as f32).min(1.0);
            let e = ease_out(t);
            let (l, tp) = (lerp(f.from.left, f.to.left, e), lerp(f.from.top, f.to.top, e));
            let (r, b) = (lerp(f.from.right, f.to.right, e), lerp(f.from.bottom, f.to.bottom, e));
            hdwp = DeferWindowPos(
                hdwp,
                f.hwnd,
                HWND::default(),
                l,
                tp,
                r - l,
                b - tp,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .unwrap_or(hdwp);
            if t < 1.0 {
                live.push(f);
            }
        }
        let _ = EndDeferWindowPos(hdwp);
    }
    live
}
