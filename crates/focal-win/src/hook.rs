//! The WinEvent hook — the input side of the product, together with the
//! tabs in `tab.rs` (2026-09-03).
//!
//! `EVENT_SYSTEM_FOREGROUND` fires whenever any window becomes the
//! foreground window: click, alt-tab, taskbar, app launch. Activation was
//! the only gesture until the tabs; since 2026-09-04 it is informational
//! (`Event::Foreground` — the engine never places anything for it), and
//! it still drives the dwell timer while `dwell_ms` is on, the desktop
//! click that clears the stage, and the frames' z-order re-assert (§4).
//! `EVENT_SYSTEM_MINIMIZESTART/END` say when a window leaves for the
//! taskbar and comes back, so its frame hides and shows at once rather
//! than on the next rescan tick.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::adapter::{post, Note};

/// Callback Windows invokes for every event in the hooked range, on the
/// hook's own thread. It must stay tiny: post the window id and return,
/// so all real work happens on the main loop. Events in the range other
/// than the three we asked for (menus, mouse capture, move-size) fall
/// through the match and cost a comparison.
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
    if id_object != 0 || hwnd.is_invalid() {
        return;
    }
    let id = hwnd.0 as u64;
    match event {
        EVENT_SYSTEM_FOREGROUND => post(Note::Foreground(id)),
        EVENT_SYSTEM_MINIMIZESTART => post(Note::Minimized(id)),
        EVENT_SYSTEM_MINIMIZEEND => post(Note::Restored(id)),
        _ => {}
    }
}

/// Install the system-wide hook over the `EVENT_SYSTEM_FOREGROUND ..=
/// EVENT_SYSTEM_MINIMIZEEND` range (one hook, three events of interest).
/// `OUTOFCONTEXT` keeps the callback in our process (no DLL injection);
/// `SKIPOWNPROCESS` stops our own windows from feeding the loop.
pub fn install() -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_MINIMIZEEND,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}
