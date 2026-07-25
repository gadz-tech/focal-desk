//! The message loop that owns everything: receives raw foreground
//! notifications from the hook, applies the dwell delay, feeds the
//! engine, executes commands, drives animation ticks.
//!
//! Sketch of the loop (fills in as this layer goes live):
//!
//! ```text
//! install hook  ──►  channel(hwnd, t_activated)
//! every 16ms tick:
//!   if pending.hwnd held foreground for >= cfg.dwell_ms:
//!       for cmd in engine.handle(Promoted(hwnd)):
//!           start Flight (frame::compensate applied)
//!   anim::tick(flights, now)
//! if foreground is the shell/desktop window and dwell passes:
//!   engine.handle(ClearStage)        // clicking the desk clears the stage
//! on WM_DISPLAYCHANGE / WM_DEVICECHANGE:
//!   engine.handle(DeskMode(dock::desk_display_present()))
//! ```

use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use windows::Win32::Foundation::HWND;

static FOREGROUND_TX: OnceLock<Sender<u64>> = OnceLock::new();

/// Hand the hook a channel to post into; call once at startup.
pub fn init_channel(tx: Sender<u64>) {
    let _ = FOREGROUND_TX.set(tx);
}

/// Called from the WinEvent hook thread — must stay tiny.
pub fn post_foreground(hwnd: HWND) {
    if let Some(tx) = FOREGROUND_TX.get() {
        let _ = tx.send(hwnd.0 as u64);
    }
}
