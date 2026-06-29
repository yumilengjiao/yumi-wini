//! The layout data model: a pure, Win32-free description of how windows
//! are organized. Structure mirrors niri's conceptual model:
//!
//! ```text
//! Layout
//!  └─ Monitor        (per output)
//!      └─ Workspace  (per monitor, one active at a time)
//!          └─ Column (horizontal scroll axis)
//!              └─ Tile (vertical stack inside a column)
//! ```
//!
//! This module only manages *structure and focus indices*. Geometry
//! computation (pixel sizes, view offsets) lives in `geometry`, and
//! applying geometry to real windows lives in `win`.
//!
//! Key semantics inherited from niri:
//! - Columns form the horizontal scroll axis; each workspace scrolls
//!   independently (`view_offset` relative to the active column).
//! - A column stacks tiles vertically; the column remembers its active
//!   tile.
//! - When a new column is created and immediately removed again without
//!   focus changes, focus returns to the previously active column
//!   (`activate_prev_column_on_removal`).
//! - Column width is either a proportion of the view, or fixed pixels;
//!   `None` means "use the default width".

// Mutation/focus methods not yet wired to key bindings will be consumed
// by the input module; silence the interim dead-code warnings.
#![allow(dead_code)]

pub mod geometry;

use std::collections::HashMap;

/// Identity of a tracked window (the HWND pointer value).
pub type WindowId = isize;

/// A single window inside a column.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub id: WindowId,
    /// Height hint: proportional weight for auto height distribution.
    /// 1.0 = equal share. Fixed heights arrive later with sizing actions.
    pub height_weight: f64,
}

impl Tile {
    fn new(id: WindowId) -> Self {
        Tile {
            id,
            height_weight: 1.0,
        }
    }
}

/// How wide a column wants to be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidth {
    /// Proportion of the view width, e.g. 0.5 = half the screen.
    Proportion(f64),
    /// Fixed width in logical pixels.
    Fixed(f64),
}

/// A vertical stack of tiles; one element of the horizontal scroll axis.
#[derive(Debug, Clone)]
pub struct Column {
    pub tiles: Vec<Tile>,
    /// Index of the focused tile in this column.
    pub active_tile_idx: usize,
    /// Desired width; `None` = config default.
    pub width: Option<ColumnWidth>,
    /// Whether the column is full-width (stretched to view edges).
    pub is_full_width: bool,
}

impl Column {
    fn new(id: WindowId) -> Self {
        Column {
            tiles: vec![Tile::new(id)],
            active_tile_idx: 0,
            width: None,
            is_full_width: false,
        }
    }

    pub fn active_tile(&self) -> Option<&Tile> {
        self.tiles.get(self.active_tile_idx)
    }

    pub fn focused_id(&self) -> Option<WindowId> {
        self.active_tile().map(|t| t.id)
    }
}

/// One workspace: a horizontally scrolling sequence of columns.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub columns: Vec<Column>,
    /// Index of the focused column.
    pub active_column_idx: usize,
    /// Horizontal view offset, in pixels, relative to the position that
    /// centers the active column. Positive = view scrolled right.
    /// (Geometry module interprets this.)
    pub view_offset: f64,
    /// Niri semantics: if the active column is removed without any
    /// intermediate focus change, restore focus (and view offset) to the
    /// previously active column instead of the neighbor.
    activate_prev_column_on_removal: Option<(usize, f64)>,
}

impl Workspace {
    pub fn new() -> Self {
        Workspace {
            columns: Vec::new(),
            active_column_idx: 0,
            view_offset: 0.0,
            activate_prev_column_on_removal: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The window this workspace currently focuses.
    pub fn focused_id(&self) -> Option<WindowId> {
        self.columns.get(self.active_column_idx)?.focused_id()
    }

    /// Location (column, tile) of a window, if present.
    pub fn find(&self, id: WindowId) -> Option<(usize, usize)> {
        for (ci, col) in self.columns.iter().enumerate() {
            if let Some(ti) = col.tiles.iter().position(|t| t.id == id) {
                return Some((ci, ti));
            }
        }
        None
    }

    /// Add a window as a new column at the end (niri's default placement)
    /// and focus it.
    pub fn add_window(&mut self, id: WindowId) {
        let prev_active = if self.columns.is_empty() {
            None
        } else {
            Some((self.active_column_idx, self.view_offset))
        };
        self.columns.push(Column::new(id));
        self.active_column_idx = self.columns.len() - 1;
        self.view_offset = 0.0;
        self.activate_prev_column_on_removal = prev_active;
    }

    /// Add a window into an existing column at `column_idx` (used by
    /// absorb / move-into-column).
    pub fn add_window_to_column(&mut self, id: WindowId, column_idx: usize, tile_idx: usize) {
        let Some(col) = self.columns.get_mut(column_idx) else {
            self.add_window(id);
            return;
        };
        let tile_idx = tile_idx.min(col.tiles.len());
        col.tiles.insert(tile_idx, Tile::new(id));
        col.active_tile_idx = tile_idx;
        self.active_column_idx = column_idx;
    }

    /// Remove a window. Fixes up focus indices following niri's rules:
    /// - removing the focused column moves focus to the left neighbor
    ///   (or the prev-active column for a freshly created column),
    /// - removing a tile from the focused column keeps tile focus in
    ///   place (clamped).
    pub fn remove_window(&mut self, id: WindowId) -> bool {
        let Some((ci, ti)) = self.find(id) else {
            return false;
        };
        let col = &mut self.columns[ci];
        col.tiles.remove(ti);
        if col.tiles.is_empty() {
            self.columns.remove(ci);
            // Pick the new active column.
            let (new_idx, new_offset) = if let Some((prev_idx, prev_offset)) =
                self.activate_prev_column_on_removal
            {
                // Only valid if the removed column was the active one and
                // no focus change happened since creation.
                let candidate = prev_idx.min(self.columns.len().saturating_sub(1));
                (candidate, prev_offset)
            } else {
                (ci.saturating_sub(1), 0.0)
            };
            self.activate_prev_column_on_removal = None;
            self.active_column_idx = if self.columns.is_empty() {
                0
            } else {
                new_idx
            };
            self.view_offset = new_offset;
        } else {
            self.active_column_idx = ci;
            col.active_tile_idx = col.active_tile_idx.min(col.tiles.len() - 1);
        }
        true
    }

    /// Focus the window: sets active column and tile indices.
    pub fn focus_window(&mut self, id: WindowId) -> bool {
        let Some((ci, ti)) = self.find(id) else {
            return false;
        };
        self.active_column_idx = ci;
        self.columns[ci].active_tile_idx = ti;
        self.activate_prev_column_on_removal = None;
        true
    }

    /// Move focus one column left/right. No wraparound (niri default).
    pub fn focus_column(&mut self, dir: DirH) -> bool {
        let next = if dir == DirH::Left {
            self.active_column_idx.checked_sub(1)
        } else {
            Some(self.active_column_idx + 1).filter(|&i| i < self.columns.len())
        };
        match next {
            Some(i) if i < self.columns.len() => {
                self.active_column_idx = i;
                self.view_offset = 0.0;
                self.activate_prev_column_on_removal = None;
                true
            }
            _ => false,
        }
    }

    /// Move focus one tile up/down within the focused column.
    pub fn focus_tile(&mut self, dir: DirV) -> bool {
        let Some(col) = self.columns.get_mut(self.active_column_idx) else {
            return false;
        };
        let next = if dir == DirV::Up {
            col.active_tile_idx.checked_sub(1)
        } else {
            Some(col.active_tile_idx + 1).filter(|&i| i < col.tiles.len())
        };
        match next {
            Some(i) if i < col.tiles.len() => {
                col.active_tile_idx = i;
                true
            }
            _ => false,
        }
    }

    /// Swap the focused column with its neighbor.
    pub fn move_column(&mut self, dir: DirH) -> bool {
        if self.columns.len() < 2 {
            return false;
        }
        let i = self.active_column_idx;
        let j = if dir == DirH::Left {
            match i.checked_sub(1) {
                Some(j) => j,
                None => return false,
            }
        } else if i + 1 < self.columns.len() {
            i + 1
        } else {
            return false;
        };
        self.columns.swap(i, j);
        self.active_column_idx = j;
        true
    }

    /// Move the focused tile up/down inside its column. At the edges,
    /// the tile stays (niri's move-window-down moves it into the next
    /// column — that's `move_tile_across`).
    pub fn move_tile(&mut self, dir: DirV) -> bool {
        let Some(col) = self.columns.get_mut(self.active_column_idx) else {
            return false;
        };
        if col.tiles.len() < 2 {
            return false;
        }
        let i = col.active_tile_idx;
        let j = if dir == DirV::Up {
            match i.checked_sub(1) {
                Some(j) => j,
                None => return false,
            }
        } else if i + 1 < col.tiles.len() {
            i + 1
        } else {
            return false;
        };
        col.tiles.swap(i, j);
        col.active_tile_idx = j;
        true
    }

    /// Move the focused tile to the neighboring column (niri's
    /// move-window-to-column-left/right):
    /// - inside a multi-tile column: change position within the neighbor,
    /// - otherwise: become a standalone column at the neighbor position.
    pub fn move_tile_across(&mut self, dir: DirH) -> bool {
        let Some((ci, ti)) = self.find_focused() else {
            return false;
        };
        let neighbor = if dir == DirH::Left {
            ci.checked_sub(1)
        } else {
            Some(ci + 1).filter(|&i| i < self.columns.len())
        };
        let Some(ni) = neighbor else {
            return false;
        };

        let tile = self.columns[ci].tiles.remove(ti);
        if self.columns[ci].tiles.is_empty() {
            self.columns.remove(ci);
        } else {
            self.columns[ci].active_tile_idx =
                self.columns[ci].active_tile_idx.min(self.columns[ci].tiles.len() - 1);
        }

        if self.columns[ni].tiles.len() >= 1 {
            // Insert into the neighbor column.
            let at = if dir == DirH::Left {
                self.columns[ni].tiles.len()
            } else {
                0
            };
            self.columns[ni].tiles.insert(at, tile);
            self.columns[ni].active_tile_idx = at;
        }
        // (The standalone-column branch cannot happen: a neighbor exists,
        // so we always merge into it.)
        self.active_column_idx = ni;
        true
    }

    /// Consume or expel the focused window one slot in `dir` (niri's
    /// consume-or-expel-window-left/right):
    ///
    /// - If the focused window is alone in its column, it is *consumed*
    ///   into the adjacent column (appended at the bottom, becomes the
    ///   active tile there).
    /// - If its column holds multiple windows, it is *expelled* into a
    ///   new standalone column next to the current one.
    ///
    /// Returns false if there is no adjacent column to consume into.
    pub fn consume_or_expel(&mut self, dir: DirH) -> bool {
        let Some((ci, ti)) = self.find_focused() else {
            return false;
        };

        if self.columns[ci].tiles.len() == 1 {
            // Consume: move into the adjacent column. Check that one
            // exists before removing our column.
            let has_neighbor = match dir {
                DirH::Left => ci > 0,
                DirH::Right => ci + 1 < self.columns.len(),
            };
            if !has_neighbor {
                return false;
            }
            let width = self.columns[ci].width;
            let col = self.columns.remove(ci);
            let tile = col.tiles.into_iter().next().unwrap();
            // Column indices >= ci shifted down by one.
            let t = match dir {
                DirH::Left => ci - 1,
                DirH::Right => ci,
            };
            self.columns[t].tiles.push(tile);
            self.columns[t].active_tile_idx = self.columns[t].tiles.len() - 1;
            // Preserve the consumed column's width on the target.
            if self.columns[t].width.is_none() {
                self.columns[t].width = width;
            }
            self.active_column_idx = t;
            self.view_offset = 0.0;
            self.activate_prev_column_on_removal = None;
        } else {
            // Expel: standalone column next to the current position.
            let tile = self.columns[ci].tiles.remove(ti);
            self.columns[ci].active_tile_idx =
                self.columns[ci].active_tile_idx.min(self.columns[ci].tiles.len() - 1);
            let col = Column {
                tiles: vec![tile],
                active_tile_idx: 0,
                width: self.columns[ci].width,
                is_full_width: false,
            };
            let at = match dir {
                DirH::Left => ci,
                DirH::Right => ci + 1,
            };
            self.columns.insert(at, col);
            self.active_column_idx = at;
            self.view_offset = 0.0;
            self.activate_prev_column_on_removal = None;
        }
        true
    }

    fn find_focused(&self) -> Option<(usize, usize)> {
        let ci = self.active_column_idx;
        let ti = self.columns.get(ci)?.active_tile_idx;
        Some((ci, ti))
    }
}

/// Horizontal direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirH {
    Left,
    Right,
}

/// Vertical direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirV {
    Up,
    Down,
}

/// Per-monitor layout: a stack of workspaces, one active.
#[derive(Debug, Clone)]
pub struct MonitorLayout {
    /// Device name from `win::monitor` (stable across handle changes).
    pub device: String,
    pub workspaces: Vec<Workspace>,
    pub active_workspace_idx: usize,
    /// Fast lookup: window -> owning workspace index.
    window_map: HashMap<WindowId, usize>,
}

impl MonitorLayout {
    pub fn new(device: String) -> Self {
        MonitorLayout {
            device,
            workspaces: vec![Workspace::new()],
            active_workspace_idx: 0,
            window_map: HashMap::new(),
        }
    }

    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active_workspace_idx]
    }

    pub fn active_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active_workspace_idx]
    }

    pub fn add_window(&mut self, id: WindowId) {
        let ws = self.active_workspace_idx;
        self.active_workspace_mut().add_window(id);
        self.window_map.insert(id, ws);
    }

    pub fn remove_window(&mut self, id: WindowId) -> bool {
        let Some(ws) = self.window_map.remove(&id) else {
            return false;
        };
        self.workspaces[ws].remove_window(id)
    }

    /// Which workspace holds this window?
    pub fn workspace_of(&self, id: WindowId) -> Option<usize> {
        self.window_map.get(&id).copied()
    }
}

/// The whole layout: one entry per monitor.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub monitors: Vec<MonitorLayout>,
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn monitor(&self, device: &str) -> Option<&MonitorLayout> {
        self.monitors.iter().find(|m| m.device == device)
    }

    pub fn monitor_mut(&mut self, device: &str) -> Option<&mut MonitorLayout> {
        self.monitors.iter_mut().find(|m| m.device == device)
    }

    /// Add a monitor with a fresh workspace.
    pub fn add_monitor(&mut self, device: &str) {
        if self.monitor(device).is_none() {
            self.monitors.push(MonitorLayout::new(device.to_string()));
        }
    }

    /// Remove a monitor; its windows will need rehoming by the caller.
    pub fn remove_monitor(&mut self, device: &str) -> Option<MonitorLayout> {
        let idx = self.monitors.iter().position(|m| m.device == device)?;
        Some(self.monitors.remove(idx))
    }

    /// The window focused on a given monitor, if any.
    pub fn focused_id(&self, device: &str) -> Option<WindowId> {
        self.monitor(device)?.active_workspace().focused_id()
    }

    /// Add a window to the active workspace of a monitor.
    pub fn add_window(&mut self, device: &str, id: WindowId) {
        self.add_monitor(device);
        self.monitor_mut(device).unwrap().add_window(id);
    }

    /// Remove a window from wherever it lives. Returns the device it was
    /// found on.
    pub fn remove_window(&mut self, id: WindowId) -> Option<String> {
        for m in &mut self.monitors {
            if m.remove_window(id) {
                return Some(m.device.clone());
            }
        }
        None
    }

    /// Focus a window: activates its workspace, column and tile.
    pub fn focus_window(&mut self, id: WindowId) -> bool {
        for m in &mut self.monitors {
            if let Some(ws) = m.workspace_of(id) {
                m.active_workspace_idx = ws;
                m.active_workspace_mut().focus_window(id);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WindowId = 1;
    const B: WindowId = 2;
    const C: WindowId = 3;
    const D: WindowId = 4;

    #[test]
    fn add_and_focus() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window(B);
        ws.add_window(C);
        assert_eq!(ws.columns.len(), 3);
        assert_eq!(ws.focused_id(), Some(C));
        assert!(ws.focus_column(DirH::Left));
        assert_eq!(ws.focused_id(), Some(B));
    }

    #[test]
    fn focus_no_wraparound() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        assert!(!ws.focus_column(DirH::Left));
        assert!(!ws.focus_column(DirH::Right)); // single column
        ws.add_window(B);
        // add_window focuses the new column: B is already focused.
        assert_eq!(ws.focused_id(), Some(B));
        assert!(!ws.focus_column(DirH::Right));
        assert!(ws.focus_column(DirH::Left));
        assert_eq!(ws.focused_id(), Some(A));
        assert!(!ws.focus_column(DirH::Left));
    }

    #[test]
    fn remove_focused_column_returns_to_prev() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window(B); // B focused, prev = A
        assert_eq!(ws.focused_id(), Some(B));
        ws.remove_window(B);
        // Freshly created column removed: back to the previous column.
        assert_eq!(ws.focused_id(), Some(A));
    }

    #[test]
    fn remove_middle_column_falls_left() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window(B);
        ws.add_window(C);
        ws.focus_column(DirH::Left); // focus B
        assert_eq!(ws.focused_id(), Some(B));
        ws.remove_window(B);
        // B was focused but not "freshly created": neighbor to the left.
        assert_eq!(ws.focused_id(), Some(A));
    }

    #[test]
    fn tile_navigation_and_move() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window_to_column(B, 0, 1);
        ws.add_window_to_column(C, 0, 2);
        assert_eq!(ws.columns[0].tiles.len(), 3);
        assert_eq!(ws.focused_id(), Some(C));
        assert!(ws.focus_tile(DirV::Up));
        assert_eq!(ws.focused_id(), Some(B));
        assert!(ws.move_tile(DirV::Up));
        assert_eq!(ws.columns[0].tiles.iter().map(|t| t.id).collect::<Vec<_>>(), vec![B, A, C]);
        assert_eq!(ws.focused_id(), Some(B));
        // At top edge: no move.
        assert!(!ws.move_tile(DirV::Up));
    }

    #[test]
    fn move_column() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window(B);
        ws.add_window(C);
        // C focused (rightmost); swap with neighbor only.
        assert!(ws.move_column(DirH::Left));
        assert_eq!(
            ws.columns.iter().map(|c| c.focused_id()).collect::<Vec<_>>(),
            vec![Some(A), Some(C), Some(B)]
        );
        assert_eq!(ws.focused_id(), Some(C));
        assert!(ws.move_column(DirH::Left));
        assert_eq!(ws.focused_id(), Some(C));
        // Left edge: no move.
        assert!(!ws.move_column(DirH::Left));
        // Right edge: no move either (single-step swaps, no wrap).
        assert!(ws.move_column(DirH::Right));
        assert!(ws.move_column(DirH::Right));
        assert!(!ws.move_column(DirH::Right));
    }

    #[test]
    fn move_tile_across_merges() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window(B);
        ws.add_window_to_column(C, 0, 1); // A,C in col 0
        ws.focus_column(DirH::Right); // focus B (col 1)
        // Move B into column 0 (to its right).
        assert!(ws.move_tile_across(DirH::Left));
        assert_eq!(ws.columns.len(), 1);
        assert_eq!(
            ws.columns[0].tiles.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![A, C, B]
        );
        assert_eq!(ws.focused_id(), Some(B));
    }

    #[test]
    fn consume_and_expel() {
        let mut ws = Workspace::new();
        ws.add_window(A);
        ws.add_window(B);
        // B is alone in its column: consume-left merges it into A's
        // column, below the active tile.
        assert!(ws.consume_or_expel(DirH::Left));
        assert_eq!(ws.columns.len(), 1);
        assert_eq!(
            ws.columns[0].tiles.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![A, B]
        );
        assert_eq!(ws.focused_id(), Some(B));
        // Multi-tile column: expel always succeeds, even at the left
        // edge (standalone column at the current position).
        assert!(ws.consume_or_expel(DirH::Left));
        assert_eq!(ws.columns.len(), 2);
        assert_eq!(ws.focused_id(), Some(B));
        assert_eq!(ws.active_column_idx, 0);
        // B standalone at index 0: no left neighbor to consume into.
        assert!(!ws.consume_or_expel(DirH::Left));
        // Merge back right into A's column.
        assert!(ws.consume_or_expel(DirH::Right));
        assert_eq!(ws.columns.len(), 1);
        assert_eq!(ws.focused_id(), Some(B));
        // Expel right: standalone column after the current one.
        assert!(ws.consume_or_expel(DirH::Right));
        assert_eq!(ws.columns.len(), 2);
        assert_eq!(ws.active_column_idx, 1);
        assert_eq!(ws.focused_id(), Some(B));
        // B standalone at the right edge: no right neighbor.
        assert!(!ws.consume_or_expel(DirH::Right));
    }

    #[test]
    fn layout_add_remove_focus() {
        let mut layout = Layout::new();
        layout.add_window("DISPLAY1", A);
        layout.add_window("DISPLAY1", B);
        layout.add_window("DISPLAY2", C);
        assert_eq!(layout.focused_id("DISPLAY1"), Some(B));
        assert_eq!(layout.focused_id("DISPLAY2"), Some(C));
        assert!(layout.focus_window(A));
        assert_eq!(layout.focused_id("DISPLAY1"), Some(A));
        assert_eq!(layout.remove_window(A), Some("DISPLAY1".to_string()));
        assert_eq!(layout.focused_id("DISPLAY1"), Some(B));
        assert_eq!(layout.remove_window(C), Some("DISPLAY2".to_string()));
    }
}
