//! focal-core — the OS-independent brain of focal-desk.
//!
//! Everything in this crate is a pure function or a pure state machine:
//! no OS calls, no threads, no clocks. The operating system (via the
//! `focal-win` adapter) feeds `engine::Event`s in and executes the
//! `engine::Command`s that come out. That contract is the architecture:
//! a feature is a new Event, a new Command, or logic between the two —
//! never a hook bolted onto platform code.

pub mod config;
pub mod engine;
pub mod geometry;
pub mod layout;
pub mod wires;
