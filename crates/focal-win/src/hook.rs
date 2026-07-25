//! KEY SNIPPET 1 — the entire input side of the product is one hook.
//!
//! `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ..)` fires whenever any
//! window becomes the foreground window — click, alt-tab, taskbar,
//! app launch. That means promotion needs no gesture of its own:
//! activation IS the gesture.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

pub unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _time_ms: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        // Rule: do almost nothing in the callback. It runs with tight
        // timing constraints, and calling into the engine here would
        // mean locking. Post the hwnd to the adapter's channel; the
        // dwell timer lives in the message loop, and only when the
        // window has held focus for cfg.dwell_ms does the loop send
        // Event::Promoted(hwnd.0 as u64) into the engine.
        crate::adapter::post_foreground(hwnd);
    }
}

pub fn install() -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_win_event),
            0, // all processes
            0, // all threads
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}
