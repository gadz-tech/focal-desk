//! The log file — focal-desk's console, now that it hasn't got one.
//!
//! The service runs windowless: a console window has a resize border, so
//! focal-desk would tile its own console into a home slot and then
//! promote it when you clicked it. That leaves the tray icon to say
//! *that* it is running, and this file to say *what* it is doing.
//!
//! Sessions accumulate, separated by a dated header, so yesterday's
//! desk-mode flips are still there to read. The file is dropped once it
//! passes [`MAX_BYTES`] rather than truncated per run — a second launch
//! that bows out to the single-instance guard must not erase the running
//! service's log.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Where [`line`] appends. Set once, by [`init`].
static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Size past which the log is dropped and started over. A session is a
/// handful of lines, so this is years of history.
const MAX_BYTES: u64 = 256 * 1024;

/// Local wall-clock time as `HH:MM:SS`, straight from Win32 so the crate
/// needs no date-time dependency.
fn stamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

/// Local date as `YYYY-MM-DD`, used once in the session header.
fn today() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

/// Append raw text to the log, if one has been set up.
///
/// Opens and closes per call. The volume is a handful of lines per
/// session, and nothing sits in a buffer waiting to be lost if the
/// process is killed rather than quit. Failures are swallowed on
/// purpose: not being able to write a log is a nuisance, not a reason to
/// refuse to manage windows.
fn append(text: &str) {
    let Some(path) = PATH.get() else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().append(true).create(true).open(path) {
        let _ = file.write_all(text.as_bytes());
    }
}

/// Point the log at `path` and open a new session in it.
pub fn init(path: PathBuf) {
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        let _ = std::fs::remove_file(&path);
    }
    let _ = PATH.set(path);
    append(&format!(
        "\r\n--- focal-desk {} {} ---\r\n",
        today(),
        stamp()
    ));
}

/// Append one timestamped line.
pub fn line(msg: &str) {
    append(&format!("{}  {}\r\n", stamp(), msg));
}

/// The file currently being logged to, for the tray's "Open log" item.
pub fn path() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}
