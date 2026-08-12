//! Desk-mode detection.
//!
//! The desk is one Thunderbolt cable away from the laptop: the eGPU
//! brings up the 65" panel, and the panel appearing in the display
//! topology is what "desk mode" means. We pick the largest monitor by
//! pixel count as the managed screen, and call it a desk when it is at
//! least 5K wide — comfortably above any laptop panel, comfortably
//! below the 8K target.

use focal_core::geometry::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

/// Minimum width (in pixels) for a display to count as the desk panel.
pub const DESK_MIN_WIDTH: f32 = 5000.0;

/// Collector for the monitor enumeration callback.
struct Monitors {
    rects: Vec<Rect>,
}

/// `EnumDisplayMonitors` callback: record each monitor's virtual-desktop
/// rectangle and continue.
unsafe extern "system" fn monitor_proc(
    _monitor: HMONITOR,
    _hdc: HDC,
    rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Monitors);
    let r = *rect;
    monitors.rects.push(Rect::new(
        r.left as f32,
        r.top as f32,
        (r.right - r.left) as f32,
        (r.bottom - r.top) as f32,
    ));
    BOOL(1)
}

/// Every monitor's rectangle in virtual-desktop coordinates.
pub fn monitors() -> Vec<Rect> {
    let mut monitors = Monitors { rects: Vec::new() };
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut monitors as *mut Monitors as isize),
        );
    }
    monitors.rects
}

/// The monitor focal-desk manages (the largest one), and whether it
/// looks like the desk panel rather than the laptop's own screen.
pub fn managed_monitor() -> (Rect, bool) {
    let all = monitors();
    let best = all
        .into_iter()
        .max_by(|a, b| (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap())
        .unwrap_or(Rect::new(0.0, 0.0, 1920.0, 1080.0));
    let is_desk = best.w >= DESK_MIN_WIDTH;
    (best, is_desk)
}
