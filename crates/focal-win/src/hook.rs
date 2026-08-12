//! The foreground hook — the entire input side of the product.
//!
//! `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ..)` fires whenever any
//! window becomes the foreground window: click, alt-tab, taskbar, app
//! launch. That is why focal-desk needs no gesture of its own —
//! activation *is* the gesture.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::adapter::{post, Note};

/// Callback Windows invokes on every foreground change, on the hook's
/// own thread. It must stay tiny: post the window id and return, so all
/// real work happens on the main loop.
unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time_ms: u32,
) {
    // OBJID_WINDOW (0) only: ignore caret/menu child notifications.
    if event == EVENT_SYSTEM_FOREGROUND && id_object == 0 && !hwnd.is_invalid() {
        post(Note::Foreground(hwnd.0 as u64));
    }
}

/// Install the system-wide foreground hook. `OUTOFCONTEXT` keeps the
/// callback in our process (no DLL injection); `SKIPOWNPROCESS` stops
/// our own windows from feeding the loop.
pub fn install() -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}
