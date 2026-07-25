//! KEY SNIPPET 4 — desk-mode detection.
//!
//! The service is inert on the laptop and wakes at the desk. Two
//! sufficient signals: the 7680x4320 display is in the topology, or
//! the eGPU adapter (RTX 5060 Ti) enumerates. We check the display —
//! it's the thing that actually matters for layout — and re-check on
//! WM_DISPLAYCHANGE / WM_DEVICECHANGE delivered to a message-only
//! window, sending Event::DeskMode(bool) on every transition.

use windows::Win32::Graphics::Gdi::{
    DEVMODEW, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW, EnumDisplaySettingsW,
};
use windows::core::PCWSTR;

pub const DESK_W: u32 = 7680;
pub const DESK_H: u32 = 4320;

pub fn desk_display_present() -> bool {
    let mut i = 0u32;
    loop {
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        let ok = unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) };
        if !ok.as_bool() {
            return false;
        }
        let mut dm = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let have = unsafe {
            EnumDisplaySettingsW(PCWSTR(dd.DeviceName.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm)
        };
        if have.as_bool() && dm.dmPelsWidth == DESK_W && dm.dmPelsHeight == DESK_H {
            return true;
        }
        i += 1;
    }
}
