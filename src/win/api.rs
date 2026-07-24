//! Small helpers around the `windows` crate to keep call sites readable.

use windows::core::PWSTR;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClassNameW, GetCursorPos, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, IsWindow, IsWindowVisible, WindowFromPoint, GA_ROOT,
};
use windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX;

/// Window rect as (x, y, w, h) in screen coordinates.
pub fn window_rect(hwnd: HWND) -> Option<(f64, f64, f64, f64)> {
    let mut rc = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rc).ok()? };
    Some((
        rc.left as f64,
        rc.top as f64,
        (rc.right - rc.left) as f64,
        (rc.bottom - rc.top) as f64,
    ))
}

/// Current cursor position in screen coordinates.
pub fn cursor_pos() -> Option<(f64, f64)> {
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt).ok()? };
    Some((pt.x as f64, pt.y as f64))
}

/// The top-level (root) window at a screen point, if any. Hits on
/// child controls are resolved up to their root owner.
pub fn root_window_at(x: i32, y: i32) -> Option<HWND> {
    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.0.is_null() {
            return None;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if root.0.is_null() {
            None
        } else {
            Some(root)
        }
    }
}

/// Convert the last Win32 error into a readable string.
pub fn last_error(context: &str) -> String {
    let code = unsafe { windows::Win32::Foundation::GetLastError() };
    format!("{context} failed: Win32 error {}", code.0)
}

/// Read a window's title (caption).
pub fn window_title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }
}

/// Read a window's class name.
pub fn window_class(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let copied = GetClassNameW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }
}

/// Name of the executable that owns the window (lowercased), if it can be
/// determined.
pub fn window_exe(hwnd: HWND) -> String {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    unsafe {
        let mut pid = 0u32;
        let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            hwnd,
            Some(&mut pid),
        );
        if pid == 0 {
            return String::new();
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = windows::Win32::Foundation::CloseHandle(process);
        if ok.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .to_lowercase()
    }
}

/// Wrapper for `GetWindowLongPtrW` with the given index.
pub fn get_window_long_ptr(hwnd: HWND, index: i32) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(index)) }
}

/// True while the HWND is still a valid window.
pub fn is_alive(hwnd: HWND) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

/// True if the window is visible.
pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// Pack a pointer-sized value into `WPARAM`/`LPARAM`.
pub fn wparam(v: usize) -> WPARAM {
    WPARAM(v)
}

pub fn lparam(v: isize) -> LPARAM {
    LPARAM(v)
}

/// Close an owned handle, ignoring errors.
pub fn close_handle(handle: HANDLE) {
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
}
