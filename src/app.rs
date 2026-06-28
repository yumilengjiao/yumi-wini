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

use crate::layout::Layout;
use crate::win::events::{EventHooks, WinEvent};
use crate::win::monitor::{self, Monitor};
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
    /// Current monitor topology.
    monitors: Vec<Monitor>,
    /// The structural layout (columns/workspaces) of tracked windows.
    layout: Layout,
    /// The window the OS currently considers foreground.
    focused: Option<HWND>,
}

impl AppState {
    /// React to a decoded WinEvent; keeps the registry and layout in
    /// sync with reality. Geometry application arrives in a later
    /// module; for now the structure is maintained and logged.
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
                    self.windows.insert(info.clone());
                    self.layout_add_window(hwnd);
                }
            }
            WinEvent::Hidden(hwnd)
            | WinEvent::Cloaked(hwnd)
            | WinEvent::MinimizeStarted(hwnd) => {
                if let Some(info) = self.windows.remove(hwnd) {
                    log::info!("window hidden: \"{}\"", info.title);
                    self.layout.remove_window(hwnd.0 as isize);
                }
            }
            WinEvent::Destroyed(hwnd) => {
                if let Some(info) = self.windows.remove(hwnd) {
                    log::info!("window closed: \"{}\"", info.title);
                    self.layout.remove_window(hwnd.0 as isize);
                }
            }
            WinEvent::Foreground(hwnd) => {
                if self.focused != Some(hwnd) {
                    let title = crate::win::api::window_title(hwnd);
                    log::debug!("foreground -> {title:?}");
                    self.focused = Some(hwnd);
                    self.layout.focus_window(hwnd.0 as isize);
                }
            }
        }
    }

    /// Place a newly tracked window into the layout of the monitor it
    /// currently lives on.
    fn layout_add_window(&mut self, hwnd: HWND) {
        let id = hwnd.0 as isize;
        let device = monitor::monitor_of_window(hwnd, &self.monitors)
            .map(|m| m.device.clone())
            .unwrap_or_default();
        self.layout.add_window(&device, id);
        log::debug!(
            "layout: window {id} -> monitor {device}, column {}",
            self.layout
                .monitor(&device)
                .map(|m| m.active_workspace().columns.len())
                .unwrap_or(0)
        );
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
        let monitors = monitor::enumerate();
        for m in &monitors {
            log::info!(
                "monitor {}{}: {}x{} at ({}, {})",
                m.device,
                if m.is_primary { " (primary)" } else { "" },
                m.width(),
                m.height(),
                m.origin().0,
                m.origin().1
            );
        }

        let mut windows = WindowRegistry::new();
        let mut layout = Layout::new();
        let count = windows.adopt_existing();
        for info in windows.iter() {
            let device = monitor::monitor_of_window(info.hwnd, &monitors)
                .map(|m| m.device.clone())
                .unwrap_or_default();
            layout.add_window(&device, info.id());
        }
        log::info!("adopted {count} existing window(s) into the layout at startup");

        let state = Rc::new(RefCell::new(AppState {
            windows,
            monitors,
            layout,
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
