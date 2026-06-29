//! Geometry computation for the scrollable layout: turns the structural
//! model (columns/tiles/focus indices) into pixel rectangles.
//!
//! Coordinate systems:
//! - *Column space*: x runs left-to-right starting at 0 (the left edge of
//!   the monitor's padded content area). Column `i` occupies
//!   `[col_x[i], col_x[i] + width[i])`.
//! - *View*: the visible horizontal slice of column space. `view_pos` is
//!   the column-space x at the left edge of the content area. It equals
//!   `col_x[active] + view_offset` — the model stores the offset relative
//!   to the active column so that structural edits keep the view stable.
//!
//! The view-visibility algorithm is a port of niri's
//! `compute_new_view_offset` (see niri/src/layout/scrolling.rs): keep the
//! focused column visible with up to `gaps` padding, preferring the
//! scroll direction with less motion; center it when the source and
//! target columns can't fit together (OnOverflow mode).

use super::{ColumnWidth, WindowId, Workspace};

/// Whether / when the focused column gets centered in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CenterFocused {
    Never,
    OnOverflow,
    Always,
}

/// Layout tuning parameters (will be driven by config later).
#[derive(Debug, Clone)]
pub struct LayoutParams {
    /// Gap between columns and between tiles, in pixels.
    pub gaps: f64,
    /// Padding between the monitor work area and the content area.
    pub edge_padding: f64,
    /// Width for columns that don't specify one.
    pub default_column_width: ColumnWidth,
    pub center_focused_column: CenterFocused,
}

impl Default for LayoutParams {
    fn default() -> Self {
        // Niri defaults: gaps 8, default column width 25% of the view,
        // center focused column on overflow.
        LayoutParams {
            gaps: 8.0,
            edge_padding: 8.0,
            default_column_width: ColumnWidth::Proportion(0.25),
            center_focused_column: CenterFocused::OnOverflow,
        }
    }
}

/// A computed rectangle for one window, in absolute screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRect {
    pub id: WindowId,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Resolve a column's pixel width.
pub fn resolve_width(
    width: Option<ColumnWidth>,
    params: &LayoutParams,
    view_width: f64,
) -> f64 {
    match width.unwrap_or(params.default_column_width) {
        ColumnWidth::Proportion(p) => (p * view_width).max(1.0),
        ColumnWidth::Fixed(f) => f.max(1.0),
    }
}

/// Pixel width of each column.
pub fn column_widths(ws: &Workspace, params: &LayoutParams, view_width: f64) -> Vec<f64> {
    ws.columns
        .iter()
        .map(|c| {
            if c.is_full_width {
                view_width
            } else {
                resolve_width(c.width, params, view_width)
            }
        })
        .collect()
}

/// Column-space x of the left edge of every column.
pub fn column_xs(widths: &[f64], gaps: f64) -> Vec<f64> {
    let mut x = 0.0;
    widths
        .iter()
        .map(|&w| {
            let rv = x;
            x += w + gaps;
            rv
        })
        .collect()
}

/// The current view position (column-space x at the content-area's left
/// edge).
pub fn view_pos(ws: &Workspace, xs: &[f64]) -> f64 {
    let active = xs.get(ws.active_column_idx).copied().unwrap_or(0.0);
    active + ws.view_offset
}

/// View offset that makes `(col_x, col_width)` visible, ported from
/// niri's `compute_new_view_offset`. `cur_vp` is the current view
/// position; the returned value is relative to the target column.
pub fn view_offset_to_show(
    cur_vp: f64,
    view_width: f64,
    col_x: f64,
    col_width: f64,
    gaps: f64,
) -> f64 {
    // A column wider than the view is always left-aligned.
    if view_width <= col_width {
        return 0.0;
    }
    // Padding shrinks when the column is wide.
    let padding = ((view_width - col_width) / 2.0).clamp(0.0, gaps);
    let desired_left = col_x - padding;
    let desired_right = col_x + col_width + padding;

    // Already fully visible: leave the view where it is.
    if cur_vp <= desired_left && desired_right <= cur_vp + view_width {
        return cur_vp - col_x;
    }

    // Prefer the alignment closer to the current position.
    let dist_left = (cur_vp - desired_left).abs();
    let dist_right = ((cur_vp + view_width) - desired_right).abs();
    if dist_left <= dist_right {
        -padding
    } else {
        -(view_width - padding - col_width)
    }
}

/// View offset that centers `(col_x, col_width)` in the view.
pub fn view_offset_centered(view_width: f64, col_x: f64, col_width: f64) -> f64 {
    if view_width <= col_width {
        return 0.0;
    }
    col_x + (col_width - view_width) / 2.0
}

/// Niri's OnOverflow rule: if the source and target columns (plus a
/// neighbor for context) fit in the view together, scroll minimally;
/// otherwise center the target.
pub fn view_offset_for_column(
    ws: &Workspace,
    params: &LayoutParams,
    view_width: f64,
    xs: &[f64],
    widths: &[f64],
    idx: usize,
    prev_idx: Option<usize>,
) -> f64 {
    let target_x = xs[idx];
    let target_w = widths[idx];

    let show = |vp: f64| view_offset_to_show(vp, view_width, target_x, target_w, params.gaps);
    let centered = || view_offset_centered(view_width, target_x, target_w);

    match params.center_focused_column {
        CenterFocused::Always => centered(),
        CenterFocused::Never => show(view_pos(ws, xs)),
        CenterFocused::OnOverflow => {
            let Some(prev_idx) = prev_idx else {
                return show(view_pos(ws, xs));
            };
            if prev_idx == idx {
                return show(view_pos(ws, xs));
            }
            // Take the neighbor of the target on the side we came from.
            let source_idx = if prev_idx > idx {
                (idx + 1).min(ws.columns.len() - 1)
            } else {
                idx.saturating_sub(1)
            };
            let source_x = xs[source_idx];
            let source_w = widths[source_idx];
            let span = if source_x < target_x {
                target_x - source_x + target_w
            } else {
                source_x - target_x + source_w
            } + params.gaps * 2.0;
            if span <= view_width {
                show(view_pos(ws, xs))
            } else {
                centered()
            }
        }
    }
}

/// Clamp a raw view position so the view never scrolls beyond the first
/// or last column. When everything fits, the content is centered.
pub fn clamp_view_pos(vp: f64, xs: &[f64], widths: &[f64], view_width: f64) -> f64 {
    let Some((&first_x, _)) = xs.first().zip(widths.first()) else {
        return vp;
    };
    let last_right = xs.last().unwrap() + widths.last().unwrap();
    let total = last_right - first_x;
    if total <= view_width {
        // Center the whole content.
        first_x - (view_width - total) / 2.0
    } else {
        vp.clamp(first_x, last_right - view_width)
    }
}

/// Distribute the column's height among its tiles by weight, honoring
/// `gaps` between tiles. Returns `(y, height)` pairs in column-space.
pub fn tile_heights(ws: &Workspace, ci: usize, params: &LayoutParams, area_h: f64) -> Vec<(f64, f64)> {
    let col = &ws.columns[ci];
    let n = col.tiles.len();
    if n == 0 {
        return Vec::new();
    }
    let available = (area_h - params.gaps * (n - 1) as f64).max(n as f64);
    let total_weight: f64 = col.tiles.iter().map(|t| t.height_weight).sum();
    let mut out = Vec::with_capacity(n);
    let mut y = 0.0;
    let mut used = 0.0;
    for (i, tile) in col.tiles.iter().enumerate() {
        let share = if i == n - 1 {
            // Last tile takes the remainder to avoid rounding loss.
            available - used
        } else {
            let h = available * tile.height_weight / total_weight;
            used += h;
            h
        };
        out.push((y, share.max(1.0)));
        y += share + params.gaps;
    }
    out
}

/// Compute the full on-screen geometry of a workspace, in absolute
/// screen coordinates. `area` is the monitor work rect
/// (left, top, width, height).
pub fn compute_workspace_geometry(
    ws: &Workspace,
    params: &LayoutParams,
    area: (f64, f64, f64, f64),
) -> Vec<TileRect> {
    let (ax, ay, aw, ah) = area;
    let view_width = (aw - params.edge_padding * 2.0).max(1.0);
    let view_height = (ah - params.edge_padding * 2.0).max(1.0);

    let widths = column_widths(ws, params, view_width);
    let xs = column_xs(&widths, params.gaps);
    let vp = clamp_view_pos(view_pos(ws, &xs), &xs, &widths, view_width);

    // Left edge of the content area in screen coordinates.
    let origin_x = ax + params.edge_padding;
    let origin_y = ay + params.edge_padding;

    let mut out = Vec::new();
    for (ci, col) in ws.columns.iter().enumerate() {
        let screen_x = origin_x + (xs[ci] - vp);
        let heights = tile_heights(ws, ci, params, view_height);
        for (ti, &(y, h)) in heights.iter().enumerate() {
            let tile = &col.tiles[ti];
            out.push(TileRect {
                id: tile.id,
                x: (origin_x + (screen_x - origin_x)).round() as i32,
                y: (origin_y + y).round() as i32,
                w: widths[ci].round().max(1.0) as i32,
                h: h.round().max(1.0) as i32,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{DirH, Workspace};
    use super::*;

    const W: f64 = 1000.0;
    const H: f64 = 800.0;

    fn params() -> LayoutParams {
        LayoutParams {
            gaps: 8.0,
            edge_padding: 8.0,
            default_column_width: ColumnWidth::Proportion(0.25),
            center_focused_column: CenterFocused::OnOverflow,
        }
    }

    fn area() -> (f64, f64, f64, f64) {
        (0.0, 0.0, W, H)
    }

    #[test]
    fn widths_default_and_overrides() {
        let mut ws = Workspace::new();
        ws.add_window(1);
        ws.add_window(2);
        ws.columns[1].width = Some(ColumnWidth::Fixed(300.0));
        let p = params();
        let ws_w = W - 16.0;
        let widths = column_widths(&ws, &p, ws_w);
        assert_eq!(widths, vec![0.25 * ws_w, 300.0]);
        let xs = column_xs(&widths, 8.0);
        assert_eq!(xs, vec![0.0, 0.25 * ws_w + 8.0]);
    }

    #[test]
    fn view_pos_is_relative_to_active_column() {
        let mut ws = Workspace::new();
        ws.add_window(1);
        ws.add_window(2);
        let p = params();
        let widths = column_widths(&ws, &p, W);
        let xs = column_xs(&widths, p.gaps);
        assert_eq!(view_pos(&ws, &xs), xs[1]); // active = col 1, offset 0
        ws.view_offset = -10.0;
        assert_eq!(view_pos(&ws, &xs), xs[1] - 10.0);
    }

    #[test]
    fn offset_when_already_visible() {
        // Column at x=500 width 200, view 1000 wide, currently at 0:
        // fully visible -> view stays.
        let off = view_offset_to_show(0.0, 1000.0, 500.0, 200.0, 8.0);
        assert_eq!(off, 0.0 - 500.0);
    }

    #[test]
    fn offset_prefers_less_motion() {
        // Column at x=1200 width 200; view at 0. Desired right edge:
        // 1200+200+8 = 1408 > 1000 -> must move. Right-align needs
        // vp = 1408-1000 = 408; left-align vp = 1192. Right is closer.
        let off = view_offset_to_show(0.0, 1000.0, 1200.0, 200.0, 8.0);
        assert_eq!(off, -(1000.0 - 8.0 - 200.0)); // right-aligned: -792
    }

    #[test]
    fn wide_column_left_aligns() {
        let off = view_offset_to_show(300.0, 500.0, 100.0, 800.0, 8.0);
        assert_eq!(off, 0.0);
    }

    #[test]
    fn centered_offset() {
        // Centering the column at x=400: view pos = col_x - (view-col)/2.
        assert_eq!(view_offset_centered(1000.0, 400.0, 200.0), 0.0);
        assert_eq!(view_offset_centered(1000.0, 0.0, 200.0), -400.0);
    }

    #[test]
    fn clamp_centers_when_content_fits() {
        let xs = vec![0.0, 260.0];
        let widths = vec![250.0, 250.0];
        // Total 510 < 1000: centered.
        let vp = clamp_view_pos(0.0, &xs, &widths, 1000.0);
        assert_eq!(vp, -(1000.0 - 510.0) / 2.0);
        // Content larger than the view: clamped to [0, last_right - view].
        let xs2 = vec![0.0, 2000.0];
        let widths2 = vec![500.0, 500.0];
        assert_eq!(clamp_view_pos(-50.0, &xs2, &widths2, 1000.0), 0.0);
        assert_eq!(clamp_view_pos(3000.0, &xs2, &widths2, 1000.0), 1500.0);
    }

    #[test]
    fn tile_heights_split_by_weight() {
        let mut ws = Workspace::new();
        ws.add_window(1);
        ws.add_window_to_column(2, 0, 1);
        let p = params();
        let hs = tile_heights(&ws, 0, &p, 800.0);
        let (y0, h0) = hs[0];
        let (y1, h1) = hs[1];
        assert_eq!(y0, 0.0);
        assert_eq!(h0, (800.0 - 8.0) / 2.0);
        assert_eq!(y1, h0 + 8.0);
        assert_eq!(h1, (800.0 - 8.0) / 2.0);
    }

    #[test]
    fn full_geometry_three_columns() {
        let mut ws = Workspace::new();
        ws.add_window(1);
        ws.add_window(2);
        ws.add_window(3);
        let p = params();
        let rects = compute_workspace_geometry(&ws, &p, area());
        assert_eq!(rects.len(), 3);
        let content_w = W - 16.0;
        let col_w = 0.25 * content_w;
        // Content (3 cols + gaps = 3*234+16..) is centered since it fits.
        let total = 3.0 * col_w + 16.0;
        let vp = -(content_w - total) / 2.0;
        for (i, r) in rects.iter().enumerate() {
            assert_eq!(r.w, col_w as i32);
            let expected_x = (8.0 + (i as f64 * (col_w + 8.0) - vp)).round() as i32;
            assert_eq!(r.x, expected_x);
            assert_eq!(r.y, 8);
            assert_eq!(r.h, H as i32 - 16);
        }
    }

    #[test]
    fn focus_far_column_scrolls_view() {
        let mut ws = Workspace::new();
        for i in 1..=8 {
            ws.add_window(i);
        }
        // All columns have default width; total >> view.
        let p = params();
        let view_w = W - 16.0;
        let widths = column_widths(&ws, &p, view_w);
        let xs = column_xs(&widths, p.gaps);
        // Focus the last column.
        ws.focus_column(DirH::Right);
        ws.focus_column(DirH::Right);
        ws.focus_column(DirH::Right);
        ws.focus_column(DirH::Right);
        ws.focus_column(DirH::Right);
        ws.focus_column(DirH::Right);
        ws.focus_column(DirH::Right);
        let prev = Some(6usize);
        let new_off = view_offset_for_column(&ws, &p, view_w, &xs, &widths, 7, prev);
        let vp = xs[7] + new_off;
        // The last column must be fully visible.
        assert!(vp <= xs[7], "left edge visible: {vp} <= {}", xs[7]);
        assert!(xs[7] + widths[7] <= vp + view_w, "right edge visible");
    }
}
