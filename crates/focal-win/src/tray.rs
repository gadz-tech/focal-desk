//! The notification-area icon: the only visible sign the service exists.
//!
//! It hangs off the hidden window the adapter already creates, so there
//! is no second window and no second message loop — Shell_NotifyIcon
//! just posts [`CALLBACK`] to the same `wnd_proc` that already handles
//! hotkeys and display changes.
//!
//! Two Win32 traps are handled here:
//! - the icon must be *removed* on quit, or a dead one lingers in the
//!   tray until the user happens to hover it;
//! - `SetForegroundWindow` must be called before `TrackPopupMenu`, or the
//!   menu refuses to dismiss when the user clicks away from it.

use std::os::windows::ffi::OsStrExt;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, SetForegroundWindow,
    TrackPopupMenu, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

/// The message Shell_NotifyIcon posts to our hidden window when the user
/// interacts with the icon. `lParam` carries the mouse message.
pub const CALLBACK: u32 = WM_APP + 1;

/// Our single icon's id within this window.
const ICON_ID: u32 = 1;

/// Menu command: empty the focal stage (same effect as Ctrl+Alt+Space).
pub const CMD_CLEAR: u32 = 1;
/// Menu command: open the log file in whatever handles .log.
pub const CMD_LOG: u32 = 2;
/// Menu command: shut the service down cleanly.
pub const CMD_QUIT: u32 = 3;

/// Copy `text` into a fixed wide buffer, truncating and NUL-terminating.
fn fill(dst: &mut [u16], text: &str) {
    let src: Vec<u16> = text.encode_utf16().take(dst.len() - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}

/// The parts of the struct that identify *our* icon, shared by every
/// Shell_NotifyIcon call.
fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

/// Put the icon in the tray. Uses the stock application icon, so there
/// is no `.ico` asset to embed and no resource-compiler dependency.
///
/// Returns whether the shell accepted it — worth logging, because a
/// missing icon is otherwise indistinguishable from a service that never
/// started, which is the exact question the icon exists to answer.
pub fn add(hwnd: HWND, tip: &str) -> bool {
    unsafe {
        let mut data = base(hwnd);
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = CALLBACK;
        if let Ok(icon) = LoadIconW(None, IDI_APPLICATION) {
            data.hIcon = icon;
        }
        fill(&mut data.szTip, tip);
        Shell_NotifyIconW(NIM_ADD, &data).as_bool()
    }
}

/// Update the hover text. Called only when the state it reports actually
/// changes, not on every poll.
pub fn set_tip(hwnd: HWND, tip: &str) {
    unsafe {
        let mut data = base(hwnd);
        data.uFlags = NIF_TIP;
        fill(&mut data.szTip, tip);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Take the icon out of the tray. Must run before the process exits.
pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &base(hwnd));
    }
}

/// Show the right-click menu at the cursor and return the chosen command
/// (one of the `CMD_*` constants), or 0 if the user dismissed it.
pub fn show_menu(hwnd: HWND) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let _ = AppendMenuW(menu, MF_STRING, CMD_CLEAR as usize, w!("Clear stage"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_LOG as usize, w!("Open log"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT as usize, w!("Quit focal-desk"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Without this the menu stays on screen after you click away.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        chosen.0 as u32
    }
}

/// Open the log file with its default handler, for the menu item.
pub fn open_log() {
    let Some(path) = crate::log::path() else {
        return;
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}
