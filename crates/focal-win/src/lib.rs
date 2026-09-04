//! focal-win — the Windows adapter.
//!
//! Its whole job is translation: Win32 happenings become
//! `focal_core::engine::Event`s, and the `Command`s that come back
//! become real window moves. The interesting logic lives in focal-core;
//! the interesting *hazards* live here:
//!
//! - [`hook`]    — the WinEvent foreground hook (the entire input side)
//! - [`frame`]   — DWM invisible-border compensation
//! - [`anim`]    — the ease-out flight between rectangles
//! - [`dock`]    — desk-mode detection (is the big panel present?)
//! - [`win`]     — small safe wrappers over the raw Win32 calls
//! - [`log`]     — the log file, since the service has no console
//! - [`snapshot`] — the flying likeness: a DWM thumbnail moves, the window is resized once (2026-09-04)
//! - [`tray`]    — the notification-area icon and its menu
//! - [`tab`]     — the tap targets: tap to promote, drag to move (2026-09-03)
//! - [`adapter`] — the service loop that ties it together

#[cfg(windows)]
pub mod adapter;
#[cfg(windows)]
pub mod anim;
#[cfg(windows)]
pub mod dock;
#[cfg(windows)]
pub mod frame;
#[cfg(windows)]
pub mod hook;
#[cfg(windows)]
pub mod log;
#[cfg(windows)]
pub mod snapshot;
#[cfg(windows)]
pub mod tab;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod win;

/// True when compiled for the platform this adapter targets.
pub fn platform_ready() -> bool {
    cfg!(windows)
}
