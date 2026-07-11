//! Top-level application shell: owns the Win32 message loop that drives
//! window tracking, input handling and (later) animations.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostQuitMessage, PostThreadMessageW, TranslateMessage, MSG,
    WM_QUIT,
};

use crate::anim::Animator;
use crate::config::{self, Action, Config};
use crate::input;
use crate::layout::geometry::{self, LayoutParams};
use crate::layout::{DirH, DirV, Edge, Layout, SizeChange};
use crate::win::events::{EventHooks, WinEvent};
use crate::win::monitor::{self, Monitor};
use crate::win::msg_window::MessageWindow;
use crate::win::placement;
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

/// A floating window: freed from the tiling grid, placed freely.
/// Niri remembers the floating position per window while it floats;
/// toggling back re-tiles it.
#[derive(Debug, Clone)]
struct FloatState {
    /// Which monitor / workspace the float belongs to.
    device: String,
    workspace_idx: usize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Mutable state shared between the message loop and event handlers.
/// (The config field is consumed by the input module in the next
/// commits.)
#[allow(dead_code)]
struct AppState {
    /// All top-level application windows we currently track.
    windows: WindowRegistry,
    /// Current monitor topology.
    monitors: Vec<Monitor>,
    /// The structural layout (columns/workspaces) of tracked windows.
    layout: Layout,
    /// Layout tuning parameters (config-driven).
    params: LayoutParams,
    /// Full configuration (binds, mod key).
    config: Config,
    /// The window the OS currently considers foreground.
    focused: Option<HWND>,
    /// While the user drags/resizes this window, tiling is paused so we
    /// don't fight the user's mouse.
    interacting_window: Option<HWND>,
    /// Windows we stripped decorations from (windowed fullscreen);
    /// used to restore them on exit/removal.
    borderless: std::collections::HashSet<isize>,
    /// Floating windows (out of the tiling grid).
    floating: std::collections::HashMap<isize, FloatState>,
    /// Workspace indicator overlay (also relays WM_DISPLAYCHANGE).
    overlay: Option<crate::win::overlay::OverlayWindow>,
    /// Window geometry animations.
    animator: Animator,
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
                    self.reflow();
                }
            }
            WinEvent::Hidden(hwnd)
            | WinEvent::Cloaked(hwnd)
            | WinEvent::MinimizeStarted(hwnd) => {
                if let Some(info) = self.windows.remove(hwnd) {
                    log::info!("window hidden: \"{}\"", info.title);
                    let id = hwnd.0 as isize;
                    self.layout.remove_window(id);
                    self.animator.remove(id);
                    self.restore_borders(id);
                    self.floating.remove(&id);
                    self.reflow();
                }
            }
            WinEvent::Destroyed(hwnd) => {
                if let Some(info) = self.windows.remove(hwnd) {
                    log::info!("window closed: \"{}\"", info.title);
                    let id = hwnd.0 as isize;
                    self.layout.remove_window(id);
                    self.animator.remove(id);
                    self.restore_borders(id);
                    self.floating.remove(&id);
                    self.reflow();
                }
            }
            WinEvent::MoveSizeStart(hwnd) => {
                if self.windows.contains(hwnd) {
                    log::debug!("user interaction started on window {id}", id = hwnd.0 as isize);
                    self.interacting_window = Some(hwnd);
                }
            }
            WinEvent::MoveSizeEnd(hwnd) => {
                if self.interacting_window == Some(hwnd) {
                    log::debug!("user interaction ended on window {id}", id = hwnd.0 as isize);
                    self.interacting_window = None;
                    let id = hwnd.0 as isize;
                    if let Some(fs) = self.floating.get_mut(&id) {
                        // The user moved/resized a floating window: adopt
                        // the new rect instead of snapping it back.
                        if let Some((x, y, w, h)) = crate::win::api::window_rect(hwnd) {
                            fs.x = x;
                            fs.y = y;
                            fs.w = w;
                            fs.h = h;
                        }
                    } else {
                        // Let the user's drag win for now: re-tile
                        // everything back to the layout.
                        self.reflow();
                    }
                }
            }
            WinEvent::Foreground(hwnd) => {
                if self.focused != Some(hwnd) {
                    self.focused = Some(hwnd);
                    let id = hwnd.0 as isize;
                    if self.layout.focus_window(id) {
                        self.update_focus_view(id);
                        self.reflow();
                    }
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

    /// Scroll the workspace owning `id` so the newly focused column is
    /// visible (instantly for now; animations come later).
    fn update_focus_view(&mut self, id: crate::layout::WindowId) {
        let params = self.params.clone();
        for m in &mut self.layout.monitors {
            if let Some(ws_idx) = m.workspace_of(id) {
                let Some(mon) = self.monitors.iter().find(|mon| mon.device == m.device) else {
                    continue;
                };
                let view_width = (mon.width() as f64 - params.edge_padding * 2.0).max(1.0);
                let ws = &mut m.workspaces[ws_idx];
                geometry::refresh_view_offset(ws, &params, view_width, None);
                return;
            }
        }
    }

    /// Execute a bound action. The navigation subset works on the
    /// focused window's workspace.
    fn dispatch(&mut self, action: Action) {
        use Action::*;
        // Find the device of the focused window (fallback: monitor
        // under the cursor).
        let focused_id = self
            .focused
            .map(|h| h.0 as isize)
            .or_else(|| {
                let cursor_mon = monitor::monitor_at_cursor(&self.monitors)?;
                self.layout
                    .monitor(&cursor_mon.device)
                    .and_then(|m| m.active_workspace().focused_id())
            });

        let Some(id) = focused_id else { return };

        // Floating windows are not in the tiling: handle the small
        // action subset that applies to them directly.
        if self.floating.contains_key(&id) {
            if matches!(action, Action::ToggleWindowFloating) {
                let fs = self.floating.get(&id).cloned().unwrap();
                self.unfloat_window(id, fs);
                return;
            }
            // Map a few tiling actions to float move/resize.
            let mut touched = false;
            if let Some(fs) = self.floating.get_mut(&id) {
                const STEP: f64 = 50.0;
                match action {
                    Action::MoveColumnLeft => {
                        fs.x -= STEP;
                        touched = true;
                    }
                    Action::MoveColumnRight => {
                        fs.x += STEP;
                        touched = true;
                    }
                    Action::MoveWindowUp => {
                        fs.y -= STEP;
                        touched = true;
                    }
                    Action::MoveWindowDown => {
                        fs.y += STEP;
                        touched = true;
                    }
                    Action::SetColumnWidth(spec) => {
                        if let Some(change) = SizeChange::parse(&spec) {
                            match change {
                                SizeChange::Delta(d) => fs.w = (fs.w + d).max(100.0),
                                SizeChange::Fixed(f) => fs.w = f.max(100.0),
                                SizeChange::Proportion(p) => {
                                    fs.w = (fs.w * p.clamp(0.05, 20.0)).max(100.0)
                                }
                            }
                            touched = true;
                        }
                    }
                    Action::SetWindowHeight(spec) => {
                        if let Some(change) = SizeChange::parse(&spec) {
                            match change {
                                SizeChange::Delta(d) => fs.h = (fs.h + d).max(100.0),
                                SizeChange::Fixed(f) => fs.h = f.max(100.0),
                                SizeChange::Proportion(p) => {
                                    fs.h = (fs.h * p.clamp(0.05, 20.0)).max(100.0)
                                }
                            }
                            touched = true;
                        }
                    }
                    _ => {}
                }
                // Keep at least 100 px of the float on its monitor.
                if touched {
                    if let Some(m) = self.monitors.iter().find(|m| m.device == fs.device) {
                        fs.x = fs
                            .x
                            .clamp(m.work.left as f64 - fs.w + 100.0, m.work.right as f64 - 100.0);
                        fs.y = fs
                            .y
                            .clamp(m.work.top as f64 - fs.h + 100.0, m.work.bottom as f64 - 100.0);
                    }
                }
            }
            if touched {
                self.reflow();
            }
            // Other tiling actions fall through to the tiling below.
            return;
        }
        // Tiling -> float happens before the layout lookup too (the
        // window leaves the layout immediately).
        if matches!(action, Action::ToggleWindowFloating) {
            self.float_window(id);
            return;
        }

        let Some(device) = self
            .layout
            .monitors
            .iter()
            .find_map(|m| m.workspace_of(id).map(|_| m.device.clone()))
        else {
            return;
        };

        let mut changed = false;
        // Windowed-fullscreen bookkeeping, applied after the layout
        // borrow ends (see below).
        let mut fullscreen_prev: Option<isize> = None;
        let mut fullscreen_now: Option<isize> = None;
        // Workspace we switched to (for the indicator overlay), if any.
        let mut ws_switch: Option<usize> = None;
        {
            let monitor_layout = self.layout.monitor_mut(&device).unwrap();
            let ws = monitor_layout.active_workspace_mut();
            match action {
                FocusColumnLeft => changed = ws.focus_column(DirH::Left),
                FocusColumnRight => changed = ws.focus_column(DirH::Right),
                FocusWindowDown => changed = ws.focus_tile(DirV::Down),
                FocusWindowUp => changed = ws.focus_tile(DirV::Up),
                MoveColumnLeft => changed = ws.move_column(DirH::Left),
                MoveColumnRight => changed = ws.move_column(DirH::Right),
                MoveWindowDown => changed = ws.move_tile(DirV::Down),
                MoveWindowUp => changed = ws.move_tile(DirV::Up),
                MoveWindowToColumnLeft => changed = ws.move_tile_across(DirH::Left),
                MoveWindowToColumnRight => changed = ws.move_tile_across(DirH::Right),
                ConsumeOrExpelWindowLeft => changed = ws.consume_or_expel(DirH::Left),
                ConsumeOrExpelWindowRight => changed = ws.consume_or_expel(DirH::Right),
                FocusColumnFirst => changed = ws.focus_column_edge(Edge::First),
                FocusColumnLast => changed = ws.focus_column_edge(Edge::Last),
                SetColumnWidth(spec) => {
                    if let Some(change) = SizeChange::parse(&spec) {
                        changed = ws.set_column_width(&change);
                    }
                }
                SetWindowHeight(spec) => {
                    if let Some(change) = SizeChange::parse(&spec) {
                        changed = ws.set_window_height(&change);
                    }
                }
                ToggleFullWidth => changed = ws.toggle_full_width(),
                MaximizeColumn => changed = ws.toggle_maximized(),
                ToggleWindowedFullscreen => {
                    fullscreen_prev = ws.fullscreen_id;
                    changed = ws.toggle_fullscreen();
                    fullscreen_now = ws.fullscreen_id;
                }
                FocusWorkspace(n) | WorkspaceSwitch(n) => {
                    let idx = n.saturating_sub(1) as usize;
                    changed = monitor_layout.switch_workspace(idx);
                    if changed {
                        ws_switch = Some(idx);
                    }
                }
                MoveWindowToWorkspace(n) => {
                    let idx = n.saturating_sub(1) as usize;
                    changed = monitor_layout
                        .move_focused_window_to_workspace(idx, true)
                        .is_some();
                    if changed {
                        ws_switch = Some(idx);
                    }
                }
                MoveColumnToWorkspace(n) => {
                    let idx = n.saturating_sub(1) as usize;
                    changed = monitor_layout
                        .move_focused_column_to_workspace(idx, true)
                        .is_some();
                    if changed {
                        ws_switch = Some(idx);
                    }
                }
                Spawn(cmd) => {
                    self.spawn(&cmd);
                }
                Quit => {
                    unsafe { PostQuitMessage(0) };
                }
                CloseWindow => {
                    self.close_window(id);
                }
                _ => {}
            }
        }
        if changed {
            // Apply decoration changes for windowed fullscreen.
            if let Some(prev_id) = fullscreen_prev {
                self.restore_borders(prev_id);
            }
            if let Some(cur_id) = fullscreen_now {
                let hwnd = HWND(cur_id as *mut _);
                if crate::win::api::is_alive(hwnd) {
                    placement::set_borderless(hwnd, true);
                    placement::raise(hwnd);
                    self.borderless.insert(cur_id);
                }
            }
            self.update_focus_view(id);
            self.reflow();
            self.sync_focus_to_os();
        }
        if let Some(idx) = ws_switch {
            self.show_workspace_overlay(&device, idx);
        }
    }

    /// Flash the "Workspace N" indicator on a monitor.
    fn show_workspace_overlay(&self, device: &str, idx: usize) {
        let Some(mon) = self.monitors.iter().find(|m| m.device == device) else {
            return;
        };
        if let Some(ov) = &self.overlay {
            ov.show_workspace(mon, idx + 1);
        }
    }

    /// Display topology changed: re-enumerate monitors, rehome windows
    /// from gone monitors, adopt new ones, re-tile.
    fn on_display_change(&mut self) {
        log::info!("display topology changed; re-enumerating monitors");
        let new_monitors = monitor::enumerate();
        let new_devices: Vec<String> =
            new_monitors.iter().map(|m| m.device.clone()).collect();

        let gone: Vec<String> = self
            .monitors
            .iter()
            .map(|m| m.device.clone())
            .filter(|d| !new_devices.contains(d))
            .collect();
        for device in gone {
            if let Some(ml) = self.layout.remove_monitor(&device) {
                let ids: Vec<isize> = ml
                    .workspaces
                    .iter()
                    .flat_map(|ws| ws.window_ids())
                    .collect();
                if ids.is_empty() {
                    continue;
                }
                match self.layout.monitors.first().map(|m| m.device.clone()) {
                    Some(target) => {
                        log::info!(
                            "rehoming {} window(s) from {device} to {target}",
                            ids.len()
                        );
                        for id in ids {
                            self.layout.add_window(&target, id);
                        }
                    }
                    None => log::warn!(
                        "no monitor left; {} window(s) left in place",
                        ids.len()
                    ),
                }
            }
        }
        for m in &new_monitors {
            self.layout.add_monitor(&m.device);
        }
        self.monitors = new_monitors;
        self.reflow();
    }

    /// Restore decorations on a window we borderlessed. Safe to call
    /// for windows that no longer exist.
    fn restore_borders(&mut self, id: isize) {
        if self.borderless.remove(&id) {
            let hwnd = HWND(id as *mut _);
            if crate::win::api::is_alive(hwnd) {
                placement::set_borderless(hwnd, false);
            }
        }
    }

    /// Close a window politely (WM_CLOSE, lets apps prompt/save).
    fn close_window(&mut self, id: isize) {
        let hwnd = windows::Win32::Foundation::HWND(id as *mut _);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    /// Niri toggle-window-floating (tiling -> float): the window leaves
    /// the layout at its current position and is placed freely.
    fn float_window(&mut self, id: isize) {
        let Some(device) = self.layout.remove_window(id) else {
            return;
        };
        let ws_idx = self
            .layout
            .monitor(&device)
            .map(|m| m.active_workspace_idx)
            .unwrap_or(0);
        let hwnd = HWND(id as *mut _);
        let rect = crate::win::api::window_rect(hwnd).unwrap_or_else(|| {
            // Fallback: centered 60% of the monitor's work area.
            let mon = self
                .monitors
                .iter()
                .find(|m| m.device == device)
                .or_else(|| self.monitors.first());
            match mon {
                Some(m) => {
                    let w = (m.work.right - m.work.left) as f64 * 0.6;
                    let h = (m.work.bottom - m.work.top) as f64 * 0.6;
                    let x = m.work.left as f64 + ((m.work.right - m.work.left) as f64 - w) / 2.0;
                    let y = m.work.top as f64 + ((m.work.bottom - m.work.top) as f64 - h) / 2.0;
                    (x, y, w, h)
                }
                None => (100.0, 100.0, 800.0, 600.0),
            }
        });
        self.floating.insert(
            id,
            FloatState {
                device: device.clone(),
                workspace_idx: ws_idx,
                x: rect.0,
                y: rect.1,
                w: rect.2,
                h: rect.3,
            },
        );
        self.animator.set_target(id, rect.0, rect.1, rect.2, rect.3);
        // Close the gap in the tiling.
        self.reflow();
    }

    /// Niri toggle-window-floating (float -> tiling): back into the
    /// layout at the focused position.
    fn unfloat_window(&mut self, id: isize, fs: FloatState) {
        self.floating.remove(&id);
        self.layout.add_window(&fs.device, id);
        self.reflow();
        self.sync_focus_to_os();
    }

    /// Run a command (niri spawn). The first token is the executable,
    /// the rest is passed as parameters.
    fn spawn(&self, cmd: &str) {
        use windows::core::HSTRING;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let (app, args) = match cmd.split_once(' ') {
            Some((a, rest)) => (a, Some(rest.to_string())),
            None => (cmd, None),
        };
        let app = HSTRING::from(app);
        let args = args.map(HSTRING::from);
        let params = match &args {
            Some(a) => windows::core::PCWSTR(a.as_ptr()),
            None => windows::core::PCWSTR::null(),
        };
        let r = unsafe {
            ShellExecuteW(
                None,
                None,
                &app,
                params,
                None,
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW returns a small value (<= 32) on failure.
        if r.0 as usize <= 32 {
            log::warn!("spawn {cmd:?} failed (code {})", r.0 as isize);
        }
    }

    /// After a layout-driven focus change, tell the OS: raise the
    /// focused window.
    fn sync_focus_to_os(&mut self) {
        let Some(device) = self
            .focused
            .and_then(|h| {
                let id = h.0 as isize;
                self.layout
                    .monitors
                    .iter()
                    .find_map(|m| m.workspace_of(id).map(|_| m.device.clone()))
            })
            .or_else(|| {
                monitor::monitor_at_cursor(&self.monitors).map(|m| m.device.clone())
            })
        else {
            return;
        };
        let Some(focused_id) = self.layout.focused_id(&device) else {
            return;
        };
        let hwnd = windows::Win32::Foundation::HWND(focused_id as *mut _);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
        }
    }

    /// Recompute geometry for every monitor's active workspace and push
    /// it to the real windows (animated). Paused while the user is
    /// dragging or resizing a window.
    fn reflow(&mut self) {
        if self.interacting_window.is_some() {
            return;
        }
        let params = self.params.clone();
        let mut targets: Vec<(isize, f64, f64, f64, f64)> = Vec::new();
        let mut raise_ids: Vec<isize> = Vec::new();
        // Windows on inactive workspaces are not rendered (niri
        // semantics); windows on the active one must be visible.
        let mut hide_ids: Vec<isize> = Vec::new();
        let mut show_ids: Vec<isize> = Vec::new();
        for mon in &self.monitors {
            let Some(ml) = self.layout.monitor(&mon.device) else {
                continue;
            };
            for (i, ws) in ml.workspaces.iter().enumerate() {
                if i != ml.active_workspace_idx {
                    hide_ids.extend(ws.window_ids());
                }
            }
            let area = (
                mon.work.left as f64,
                mon.work.top as f64,
                (mon.work.right - mon.work.left) as f64,
                (mon.work.bottom - mon.work.top) as f64,
            );
            let fs_id = ml.active_workspace().fullscreen_id;
            for r in geometry::compute_workspace_geometry(ml.active_workspace(), &params, area) {
                let (x, y, w, h) = if Some(r.id) == fs_id {
                    // Windowed fullscreen: cover the whole monitor
                    // (taskbar included), not just the work area.
                    (
                        mon.full.left as f64,
                        mon.full.top as f64,
                        (mon.full.right - mon.full.left) as f64,
                        (mon.full.bottom - mon.full.top) as f64,
                    )
                } else {
                    (r.x as f64, r.y as f64, r.w as f64, r.h as f64)
                };
                targets.push((r.id, x, y, w, h));
                show_ids.push(r.id);
            }
            if let Some(fs_id) = fs_id {
                raise_ids.push(fs_id);
            }
        }
        // Floating windows: placed freely on their workspace, above
        // the tiles.
        let floats: Vec<(isize, f64, f64, f64, f64, bool)> = self
            .floating
            .iter()
            .map(|(id, fs)| {
                let visible = self
                    .layout
                    .monitor(&fs.device)
                    .map(|ml| ml.active_workspace_idx == fs.workspace_idx)
                    .unwrap_or(false);
                (*id, fs.x, fs.y, fs.w, fs.h, visible)
            })
            .collect();
        for (id, x, y, w, h, visible) in floats {
            let hwnd = HWND(id as *mut _);
            if !visible {
                if crate::win::api::is_alive(hwnd) {
                    placement::set_shown(hwnd, false);
                }
                self.animator.remove(id);
            } else {
                if crate::win::api::is_alive(hwnd) {
                    placement::set_shown(hwnd, true);
                    placement::raise(hwnd);
                }
                self.animator.set_target(id, x, y, w, h);
            }
        }
        for id in hide_ids {
            let hwnd = HWND(id as *mut _);
            if crate::win::api::is_alive(hwnd) {
                placement::set_shown(hwnd, false);
            }
            // No point animating a hidden window.
            self.animator.remove(id);
        }
        for id in &show_ids {
            let hwnd = HWND(*id as *mut _);
            if crate::win::api::is_alive(hwnd) {
                placement::set_shown(hwnd, true);
            }
        }
        for (id, x, y, w, h) in targets {
            self.animator.set_target(id, x, y, w, h);
        }
        // The fullscreen window must cover its tile siblings.
        for id in raise_ids {
            let hwnd = HWND(id as *mut _);
            if crate::win::api::is_alive(hwnd) {
                placement::raise(hwnd);
            }
        }
        // Kick an immediate frame so first paint is not delayed.
        self.tick_animations();
    }

    /// Advance all animations one frame and push geometry to windows.
    /// Called from the timer tick on the main thread.
    fn tick_animations(&mut self) {
        if !self.animator.is_animating() {
            return;
        }
        for (id, x, y, w, h) in self.animator.tick() {
            let rect = crate::layout::geometry::TileRect { id, x, y, w, h };
            placement::apply_geometry(&[rect]);
        }
    }
}

pub struct App {
    state: Rc<RefCell<AppState>>,
    /// Keeps the WinEvent hooks alive; dropping uninstalls them.
    _hooks: EventHooks,
    /// Hidden message-only window; receives marshaled key events.
    /// Kept alive for the process lifetime (field is deliberately
    /// unused after construction).
    #[allow(dead_code)]
    msg_window: MessageWindow,
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

        let cfg = config::load();

        // Animation tuning: `animations { off; }` forces zero duration,
        // which makes every retarget land instantly.
        let mut anim_params = cfg.animations.window_movement;
        if !cfg.animations.enabled {
            anim_params.duration = std::time::Duration::ZERO;
        }

        let overlay = crate::win::overlay::OverlayWindow::new();
        if overlay.is_none() {
            log::warn!("failed to create the workspace overlay window");
        }

        let state = Rc::new(RefCell::new(AppState {
            windows,
            monitors,
            layout,
            params: cfg.layout.clone(),
            config: cfg,
            focused: None,
            interacting_window: None,
            borderless: std::collections::HashSet::new(),
            floating: std::collections::HashMap::new(),
            overlay,
            animator: Animator::new(anim_params),
        }));

        // Perform the initial tiling of everything we adopted.
        state.borrow_mut().reflow();

        // The hook handler shares state with the message loop via Rc.
        // Both live on the main thread, so no locking is needed.
        let handler_state = Rc::clone(&state);
        let hooks = EventHooks::install(move |event| {
            handler_state.borrow_mut().handle_event(event);
        });

        let msg_window = MessageWindow::new()
            .ok_or_else(|| AppError("failed to create the message window".to_string()))?;

        // Keyboard: install the LL hook targeting our message window,
        // and dispatch forwarded events to bound actions.
        {
            let mod_key = state.borrow().config.mod_key.clone();
            input::install(mod_key, msg_window.hwnd())
                .map_err(AppError::from)?;
        }
        let key_state = Rc::clone(&state);
        msg_window.set_key_handler(move |ev| {
            if !ev.pressed {
                return;
            }
            let mut s = key_state.borrow_mut();
            let action = input::action_for(&s.config.binds, &ev);
            if let Some(action) = action {
                log::debug!("key action: {action:?}");
                s.dispatch(action);
            }
        });
        // Animation frames: tick the animator at ~60 Hz.
        {
            let anim_state = Rc::clone(&state);
            msg_window.start_anim_timer(move || {
                anim_state.borrow_mut().tick_animations();
            });
        }
        // Display hotplug: WM_DISPLAYCHANGE arrives on the (top-level)
        // overlay window and is forwarded here.
        if let Some(ov) = state.borrow().overlay.as_ref() {
            let dc_state = Rc::clone(&state);
            ov.set_display_change_handler(move || {
                dc_state.borrow_mut().on_display_change();
            });
        }
        // Compile the combo table for hook-side swallowing.
        input::update_binds(&state.borrow().config.binds);

        Ok(App {
            state,
            _hooks: hooks,
            msg_window,
        })
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
        input::uninstall();
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
