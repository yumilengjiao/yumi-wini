//! Top-level application shell: owns the Win32 message loop that drives
//! window tracking, input handling and (later) animations.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_QUIT,
};

use crate::win::events::{EventHooks, WinEvent};
use crate::win::window::WindowRegistry;

/// Fatal, top-level error.
#[derive(Debug)]
pub struct AppError(String);

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError(s)
    }
}

/// Mutable state shared between the message loop and event handlers.
struct AppState {
    /// All top-level application windows we currently track.
    windows: WindowRegistry,
    /// The window the OS currently considers foreground.
    focused: Option<HWND>,
}

impl AppState {
    /// React to a decoded WinEvent; keeps the registry in sync with
    /// reality. The layout engine will subscribe to these changes later.
    fn handle_event(&mut self, event: WinEvent) {
        match event {
            WinEvent::Shown(hwnd) | WinEvent::Uncloaked(hwnd) | WinEvent::MinimizeEnded(hwnd) => {
                if !self.windows.contains(hwnd) && crate::win::window::is_manageable(hwnd) {
                    let info = crate::win::window::snapshot(hwnd);
                    log::info!(
                        "window opened: [{}] \"{}\" ({})",
                        info.exe,
                        info.title,
                        info.class
                    );
                    self.windows.insert(info);
                }
            }
            WinEvent::Hidden(hwnd)
            | WinEvent::Cloaked(hwnd)
            | WinEvent::MinimizeStarted(hwnd) => {
                if let Some(info) = self.windows.remove(hwnd) {
                    log::info!("window hidden: \"{}\"", info.title);
                }
            }
            WinEvent::Destroyed(hwnd) => {
                if let Some(info) = self.windows.remove(hwnd) {
                    log::info!("window closed: \"{}\"", info.title);
                }
            }
            WinEvent::Foreground(hwnd) => {
                if self.focused != Some(hwnd) {
                    let title = crate::win::api::window_title(hwnd);
                    log::debug!("foreground -> {title:?}");
                    self.focused = Some(hwnd);
                }
            }
        }
    }
}

pub struct App {
    state: Rc<RefCell<AppState>>,
    /// Keeps the WinEvent hooks alive; dropping uninstalls them.
    _hooks: EventHooks,
}

impl App {
    /// Construct the application: adopt existing windows and install
    /// WinEvent hooks on this (main) thread.
    pub fn new() -> Result<Self, AppError> {
        let mut windows = WindowRegistry::new();
        let count = windows.adopt_existing();
        log::info!("adopted {count} existing window(s) at startup");

        let state = Rc::new(RefCell::new(AppState {
            windows,
            focused: None,
        }));

        // The hook handler shares state with the message loop via Rc.
        // Both live on the main thread, so no locking is needed.
        let handler_state = Rc::clone(&state);
        let hooks = EventHooks::install(move |event| {
            handler_state.borrow_mut().handle_event(event);
        });

        Ok(App { state, _hooks: hooks })
    }

    /// Run the Win32 message loop until a quit message arrives.
    ///
    /// WinEvent callbacks are dispatched by `GetMessageW`, so this loop is
    /// also what drives all window-tracking updates.
    pub fn run(&mut self) -> Result<(), AppError> {
        install_ctrl_c_quit();

        let mut msg = MSG::default();
        loop {
            let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if r.0 > 0 {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            } else if r.0 == 0 {
                // WM_QUIT
                break;
            } else {
                return Err(AppError(format!("GetMessageW failed: {}", r.0)));
            }
        }

        let state = self.state.borrow();
        log::info!(
            "message loop exited; was tracking {} window(s)",
            state.windows.len()
        );
        Ok(())
    }
}

/// Ctrl+C / window-close: post WM_QUIT to our own thread so the loop
/// unwinds and hooks get dropped cleanly.
fn install_ctrl_c_quit() {
    unsafe extern "system" fn handler(_ctrl_type: u32) -> windows::core::BOOL {
        let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        windows::core::BOOL(1)
    }
    unsafe {
        if SetConsoleCtrlHandler(Some(handler), true).is_err() {
            log::warn!("failed to install Ctrl+C handler");
        }
    }
}
