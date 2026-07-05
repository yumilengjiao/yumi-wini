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
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, RegisterClassW,
    SetTimer, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW, GWLP_USERDATA,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
};

use crate::input::{self, KeyEvent};

/// Custom message: a key event forwarded from the keyboard hook.
pub const WM_APP_KEY: u32 = 0x8000; // WM_APP
/// Custom message: a mouse event forwarded from a hook.
pub const WM_APP_MOUSE: u32 = 0x8001;
/// Animation frame timer id.
pub const TIMER_ANIM: usize = 1;
/// Animation frame period (ms). ~60 Hz.
pub const ANIM_TIMER_MS: u32 = 16;

type KeyHandler = Box<dyn Fn(KeyEvent)>;
type TimerHandler = Box<dyn Fn()>;

/// The single instance owning the key handler; wnd_proc needs to reach
/// it from a static context (single-threaded: all on the main thread).
/// Wrapped in a struct with an unsafe Sync impl instead of `static mut`
/// (banned to reference in edition 2024).
struct HandlerCell(std::cell::UnsafeCell<Option<KeyHandler>>);
unsafe impl Sync for HandlerCell {}

struct TimerHandlerCell(std::cell::UnsafeCell<Option<TimerHandler>>);
unsafe impl Sync for TimerHandlerCell {}

impl HandlerCell {
    const fn new() -> Self {
        HandlerCell(std::cell::UnsafeCell::new(None))
    }

    /// Safety: only call from the main thread.
    unsafe fn set(&self, handler: KeyHandler) {
        unsafe { *self.0.get() = Some(handler) }
    }

    /// Safety: only call from the main thread; the returned reference
    /// must not outlive the next `set` call.
    unsafe fn get(&self) -> Option<&KeyHandler> {
        unsafe { (*self.0.get()).as_ref() }
    }

    /// Safety: only call from the main thread.
    unsafe fn take(&self) -> Option<KeyHandler> {
        unsafe { (*self.0.get()).take() }
    }
}

static KEY_HANDLER: HandlerCell = HandlerCell::new();
static TIMER_HANDLER: TimerHandlerCell = TimerHandlerCell::new();

impl TimerHandlerCell {
    const fn new() -> Self {
        TimerHandlerCell(std::cell::UnsafeCell::new(None))
    }

    /// Safety: main thread only.
    unsafe fn set(&self, handler: TimerHandler) {
        unsafe { *self.0.get() = Some(handler) }
    }

    /// Safety: main thread only.
    unsafe fn get(&self) -> Option<&TimerHandler> {
        unsafe { (*self.0.get()).as_ref() }
    }

    /// Safety: main thread only.
    unsafe fn take(&self) -> Option<TimerHandler> {
        unsafe { (*self.0.get()).take() }
    }
}

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

    /// Set the handler invoked (on the main thread, during message
    /// dispatch) for each key event forwarded by the keyboard hook.
    pub fn set_key_handler(&self, handler: impl Fn(KeyEvent) + 'static) {
        // Safety: main thread only; wnd_proc runs on the main thread
        // during message dispatch.
        unsafe {
            KEY_HANDLER.set(Box::new(handler));
        }
    }

    /// Set the periodic animation tick handler and start the timer.
    pub fn start_anim_timer(&self, handler: impl Fn() + 'static) {
        unsafe {
            TIMER_HANDLER.set(Box::new(handler));
            let _ = SetTimer(Some(self.hwnd), TIMER_ANIM, ANIM_TIMER_MS, None);
        }
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = KEY_HANDLER.take();
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
    if msg == WM_APP_KEY {
        // Safety: reads the handler on the same (main) thread that set
        // it, outside any mutation window.
        unsafe {
            if let Some(handler) = KEY_HANDLER.get() {
                let ev = input::decode_message(wparam.0, lparam.0);
                // Never let a panic cross the FFI boundary.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(ev)));
            }
        }
        return LRESULT(0);
    }
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_TIMER
        && wparam.0 == TIMER_ANIM
    {
        unsafe {
            if let Some(handler) = TIMER_HANDLER.get() {
                let _ =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler()));
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Convenience: build a PCWSTR from a static utf16 literal is done via
/// windows::core::w; this helper exists for dynamic names later.
#[allow(dead_code)]
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[allow(dead_code)]
pub fn pcwstr(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}

#[allow(dead_code)]
pub fn window_user_data(hwnd: HWND) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
}

#[allow(dead_code)]
pub fn set_window_user_data(hwnd: HWND, value: isize) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, value);
    }
}
