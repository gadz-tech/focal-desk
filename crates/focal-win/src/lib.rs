//! focal-win — the Windows adapter.
//!
//! This crate's whole job is translation: real Win32 happenings become
//! `focal_core::engine::Event`s, and `Command`s coming back become
//! actual window moves. The interesting logic lives in focal-core; the
//! interesting *dangers* live here, documented per-module:
//!
//! - [`hook`]  — the WinEvent foreground hook (the entire input side)
//! - [`frame`] — DWM invisible-border compensation (why naive
//!   SetWindowPos leaves ragged gutters)
//! - [`anim`]  — the ease-out flight between rects
//! - [`dock`]  — desk-mode detection (eGPU / 65" present?)
//! - [`adapter`] — the message loop tying it together, incl. the dwell
//!   timer that turns raw focus changes into `Promoted` events
//!
//! Modules are gated behind the `win32` feature (see Cargo.toml) so the
//! workspace builds and tests everywhere today; enable it from a
//! Windows machine when it's time to run for real.

#[cfg(all(windows, feature = "win32"))]
pub mod adapter;
#[cfg(all(windows, feature = "win32"))]
pub mod anim;
#[cfg(all(windows, feature = "win32"))]
pub mod dock;
#[cfg(all(windows, feature = "win32"))]
pub mod frame;
#[cfg(all(windows, feature = "win32"))]
pub mod hook;

/// True when compiled for the platform this adapter targets.
pub fn platform_ready() -> bool {
    cfg!(windows)
}
