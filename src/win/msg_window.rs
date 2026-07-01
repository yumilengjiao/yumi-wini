//! Hidden message-only window.
//!
//! The main thread needs an HWND to receive posted messages: the
//! low-level keyboard hook (running on a hook thread) marshals key
//! events here via `PostMessage`, and timers can be attached to it
//! later for driving animations.

use windows::core::w;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
};

/// Custom message: a key event forwarded from the keyboard hook.
pub const WM_APP_KEY: u32 = 0x8000; // WM_APP
/// Custom message: a mouse event forwarded from a hook.
pub const WM_APP_MOUSE: u32 = 0x8001;

pub struct MessageWindow {
    hwnd: HWND,
}

impl MessageWindow {
    /// Create a message-only window owned by the calling thread. The
    /// thread must pump messages for deliveries to arrive.
    pub fn new() -> Option<Self> {
        let class_name = w!("yumi_wini_msg");

        unsafe {
            let hinstance = GetModuleHandleW(None).ok()?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            // Registering twice (e.g. in tests) fails harmlessly.
            let _ = RegisterClassW(&wc);

            // Message-only child window: never visible, never activated,
            // invisible to hits. HWND_MESSAGE = -3.
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0),
                class_name,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND(-3isize as *mut _)),
                None,
                Some(hinstance.into()),
                None,
            )
            .ok()?;

            Some(MessageWindow { hwnd })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Nothing to handle yet; key events get real handling when the
    // input module lands. Everything falls through to the default proc.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Convenience: build a PCWSTR from a static utf16 literal is done via
/// windows::core::w; this helper exists for dynamic names later.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[allow(dead_code)]
pub fn pcwstr(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}
