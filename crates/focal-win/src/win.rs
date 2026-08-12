//! Thin, safe wrappers over the Win32 calls the adapter needs.
//!
//! Every function here is deliberately tiny and does exactly one FFI
//! thing. If a signature changes between `windows` crate versions, the
//! fix is a one-line change in one place.

use std::ffi::c_void;

use focal_core::config::WindowMeta;
use focal_core::geometry::Rect;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, MAX_PATH, RECT,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetShellWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE,
    GW_OWNER, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::core::{BOOL, PWSTR};

/// Tell Windows we handle per-monitor DPI ourselves. Without this, an
/// 8K panel at any scaling factor reports virtualised coordinates and
/// every rectangle we compute lands in the wrong place.
pub fn enable_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// True when another focal-desk already holds the single-instance mutex.
///
/// Matters because there are now two ways to start: the logon task and a
/// manual launch. Two services on one desktop would fight over every
/// placement and leave two icons in the tray.
///
/// The mutex handle is deliberately never closed — it has to live as long
/// as the process, and Windows releases it on exit.
pub fn already_running() -> bool {
    unsafe {
        let existed = match CreateMutexW(None, true, windows::core::w!("focal-desk-single-instance"))
        {
            Ok(_handle) => GetLastError() == ERROR_ALREADY_EXISTS,
            // If we cannot take the mutex at all, assume we are alone
            // rather than refusing to start.
            Err(_) => false,
        };
        existed
    }
}

/// Convert a `HWND` into the engine's opaque window id.
pub fn id_of(hwnd: HWND) -> u64 {
    hwnd.0 as u64
}

/// Rebuild a `HWND` from an engine window id.
pub fn hwnd_of(id: u64) -> HWND {
    HWND(id as _)
}

/// Read a fixed-size wide buffer into a Rust `String`.
fn wide_to_string(buf: &[u16], len: usize) -> String {
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}

/// The window's title bar text (empty when it has none).
pub fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    wide_to_string(&buf, len.max(0) as usize)
}

/// The window's class name, used to recognise shell surfaces.
pub fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    wide_to_string(&buf, len.max(0) as usize)
}

/// The owning process's executable file name, e.g. `WindowsTerminal.exe`.
pub fn process_name(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false.into(), pid) else {
            return String::new();
        };
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return String::new();
        }
        wide_to_string(&buf, len as usize)
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

/// Process name plus title, as the engine's matchers expect them.
pub fn window_meta(hwnd: HWND) -> WindowMeta {
    WindowMeta {
        process: process_name(hwnd),
        title: window_title(hwnd),
    }
}

/// True when DWM reports the window as cloaked — the invisible-but-open
/// state that suspended UWP apps sit in. These must never be managed.
pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

/// Shell surfaces that look like ordinary windows but must be left alone.
const SHELL_CLASSES: [&str; 6] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
    "MultitaskingViewFrame",
];

/// True when this window is the desktop itself (clicking it clears the
/// focal stage rather than promoting anything).
pub fn is_desktop(hwnd: HWND) -> bool {
    let class = window_class(hwnd);
    hwnd == unsafe { GetShellWindow() } || class == "Progman" || class == "WorkerW"
}

/// Decide whether focal-desk should own this window's geometry.
///
/// The rules are conservative: visible, top-level, unowned, not cloaked,
/// not a tool window, not a shell surface, and actually titled. Anything
/// rejected here simply keeps Windows' normal behavior.
pub fn is_manageable(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_CHILD.0 != 0 {
            return false;
        }
        if ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
            return false;
        }
        // No resize border means the window cannot be tiled: it keeps
        // its own size wherever we put it, so a slot only ever holds it
        // wrong. Caption-less app popups and fixed dialogs land here.
        if style & WS_THICKFRAME.0 == 0 {
            return false;
        }
        // Owned windows are dialogs and palettes; they follow their owner.
        if GetWindow(hwnd, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
            return false;
        }
        if is_cloaked(hwnd) || window_title(hwnd).is_empty() {
            return false;
        }
        if SHELL_CLASSES.contains(&window_class(hwnd).as_str()) {
            return false;
        }
        let r = window_rect(hwnd);
        r.w >= 200.0 && r.h >= 120.0
    }
}

/// Collector used by [`enum_windows`]' FFI callback.
struct Collector {
    found: Vec<HWND>,
}

/// The `EnumWindows` callback: keep manageable windows, keep going.
unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let collector = &mut *(lparam.0 as *mut Collector);
    if is_manageable(hwnd) {
        collector.found.push(hwnd);
    }
    BOOL(1)
}

/// Every currently manageable top-level window, in z-order.
pub fn enum_windows() -> Vec<HWND> {
    let mut collector = Collector { found: Vec::new() };
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut collector as *mut Collector as isize),
        );
    }
    collector.found
}

/// The window rectangle as Windows reports it (includes the invisible
/// resize border — see [`crate::frame`]).
pub fn window_rect(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    Rect::new(
        r.left as f32,
        r.top as f32,
        (r.right - r.left) as f32,
        (r.bottom - r.top) as f32,
    )
}

/// The visible frame bounds according to DWM, falling back to
/// [`window_rect`] when the attribute is unavailable.
pub fn visible_rect(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if ok.is_err() {
        return window_rect(hwnd);
    }
    Rect::new(
        r.left as f32,
        r.top as f32,
        (r.right - r.left) as f32,
        (r.bottom - r.top) as f32,
    )
}

/// The monitor a window currently sits on, in virtual-desktop pixels.
pub fn monitor_rect_of(hwnd: HWND) -> Rect {
    unsafe {
        let monitor: HMONITOR = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Rect::new(0.0, 0.0, 1920.0, 1080.0);
        }
        let r = info.rcMonitor;
        Rect::new(
            r.left as f32,
            r.top as f32,
            (r.right - r.left) as f32,
            (r.bottom - r.top) as f32,
        )
    }
}
