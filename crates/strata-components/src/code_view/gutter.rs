// SPDX-License-Identifier: MIT

//! Whole-line gutter selection, Shift+click range extension, viewport-edge auto-scroll, and
//! live-only row highlights for [`super::CodeView`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use rune::{Color, Drag, Live, Phase, PointerButton, Tx};

use super::rows::Rows;
use super::state::CodeState;

/// Points per second of edge auto-scroll per point the pointer lies outside the viewport.
const EDGE_RATE: f32 = 10.0;
/// Minimum effective distance outside the viewport (80 pt/s right at the edge).
const EDGE_MIN_PAST: f32 = 8.0;
/// Maximum edge auto-scroll speed in points per second.
const EDGE_MAX_SPEED: f32 = 4000.0;
/// Maximum elapsed time integrated in a single auto-scroll tick, preventing jumps after pauses.
const EDGE_MAX_STEP_SECS: f64 = 0.05;

struct HighlightInner {
    color: Color,
    lines: Option<(usize, usize)>,
    cells: HashMap<usize, Live<Color>>,
}

/// Live-only row fills for whole-line gutter selections so dragging across lines highlights
/// every selected row (including wrapped, empty, and continuation `↳` rows) with zero view
/// commits.
#[derive(Clone)]
pub(crate) struct RowHighlights(Rc<RefCell<HighlightInner>>);

impl Default for RowHighlights {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(HighlightInner {
            color: Color::TRANSPARENT,
            lines: None,
            cells: HashMap::new(),
        })))
    }
}

impl RowHighlights {
    /// Updates the selection highlight colour from the active theme, repainting any currently
    /// highlighted mounted rows immediately.
    pub fn set_color(&self, color: Color) {
        let mut inner = self.0.borrow_mut();
        if inner.color == color {
            return;
        }
        inner.color = color;
        if inner.lines.is_some() {
            for live in inner.cells.values() {
                if live.get() != Color::TRANSPARENT {
                    live.snap(color);
                }
            }
        }
    }

    /// Returns the shared [`Live<Color>`] for display row `row` (part of logical line `line`),
    /// creating and initializing it when the row first mounts.
    pub fn live_for(&self, row: usize, line: usize) -> Live<Color> {
        let mut inner = self.0.borrow_mut();
        let want = row_fill(inner.lines, inner.color, line);
        let live = inner
            .cells
            .entry(row)
            .or_insert_with(|| Live::new(want))
            .clone();
        if live.get() != want {
            live.snap(want);
        }
        live
    }

    /// Drops `Live<Color>` handles for rows far outside the mounted window so memory stays
    /// bounded to the visible rows.
    pub fn retain(&self, keep: Range<usize>) {
        self.0.borrow_mut().cells.retain(|r, _| keep.contains(r));
    }

    /// Currently highlighted inclusive logical line range `(lo_line, hi_line)`, if any.
    pub fn lines(&self) -> Option<(usize, usize)> {
        self.0.borrow().lines
    }

    /// Updates the highlighted logical line range and snaps all mounted row fills in place.
    pub fn set_lines(&self, lines: Option<(usize, usize)>, rows: &Rows) {
        let mut inner = self.0.borrow_mut();
        if inner.lines == lines {
            return;
        }
        inner.lines = lines;
        let color = inner.color;
        for (&row_idx, live) in &inner.cells {
            let want = rows
                .get(row_idx)
                .map_or(Color::TRANSPARENT, |r| row_fill(lines, color, r.line));
            if live.get() != want {
                live.snap(want);
            }
        }
    }

    /// Clears the highlighted line range and snaps all mounted row fills to transparent.
    pub fn clear(&self) {
        let mut inner = self.0.borrow_mut();
        if inner.lines.take().is_none() {
            return;
        }
        for live in inner.cells.values() {
            if live.get() != Color::TRANSPARENT {
                live.snap(Color::TRANSPARENT);
            }
        }
    }

    /// Discards all cached row handles and selection state when the document text changes.
    pub fn reset(&self) {
        let mut inner = self.0.borrow_mut();
        inner.lines = None;
        inner.cells.clear();
    }
}

fn row_fill(lines: Option<(usize, usize)>, color: Color, line: usize) -> Color {
    match lines {
        Some((lo, hi)) if line >= lo && line <= hi => color,
        _ => Color::TRANSPARENT,
    }
}

/// Active primary-button drag originating in the line-number gutter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GutterDrag {
    pub anchor_line: usize,
    pub pointer_y: f32,
    /// Clock timestamp of the previous edge auto-scroll tick, or `None` while resting inside
    /// the viewport or at a scroll limit.
    pub last_tick: Option<f64>,
}

/// Handles a primary pointer click or drag on the persistent gutter hit strip.
pub(crate) fn gutter_drag(st: &mut CodeState, d: &mut Drag, tx: &mut Tx) {
    if tx.button() != PointerButton::Primary {
        tx.live_only();
        return;
    }
    match d.phase {
        Phase::Down => gutter_press(st, d.window_point()[1], tx),
        Phase::Move if !d.cancelled() => gutter_move(st, d.window_point()[1], tx),
        Phase::Move | Phase::Up => st.stop_gutter_drag(),
    }
    tx.live_only();
}

fn gutter_press(st: &mut CodeState, y: f32, tx: &mut Tx) {
    if tx.focused() != Some(st.view_key()) {
        tx.focus(st.view_key().to_owned(), false);
    }
    let Some(row_idx) = st.position.hit_row(y) else {
        st.on_pointer_press();
        tx.clear_text_selection();
        return;
    };
    let clicked_line = st.rows().row(row_idx).line;
    let anchor_line = if tx.mods().shift {
        shift_anchor_line(st, clicked_line, tx)
    } else {
        clicked_line
    };
    st.set_gutter_anchor(Some(anchor_line));
    st.set_gutter_drag(Some(GutterDrag {
        anchor_line,
        pointer_y: y,
        last_tick: None,
    }));
    st.apply_gutter_lines(anchor_line, clicked_line, tx);
}

fn shift_anchor_line(st: &mut CodeState, fallback: usize, tx: &Tx) -> usize {
    st.sync_caret_from_selection(tx.text_selection());
    let last = st.rows().lines().saturating_sub(1);
    if let Some(anchor) = st.gutter_anchor() {
        return anchor.min(last);
    }
    if let Some((anchor_byte, _)) = st.caret() {
        return st.rows().line_at_byte(anchor_byte);
    }
    fallback
}

fn gutter_move(st: &mut CodeState, y: f32, tx: &mut Tx) {
    let Some(mut drag) = st.gutter_drag() else {
        return;
    };
    drag.pointer_y = y;
    if edge_speed(st, y).is_some() {
        if drag.last_tick.is_none() {
            drag.last_tick = Some(tx.clock());
            tx.wake_after(Duration::ZERO);
        }
    } else {
        drag.last_tick = None;
    }
    st.set_gutter_drag(Some(drag));
    if let Some(row_idx) = st.position.drag_row(y) {
        let target_line = st.rows().row(row_idx).line;
        st.apply_gutter_lines(drag.anchor_line, target_line, tx);
    }
}

/// Steps viewport-edge auto-scrolling during an active gutter drag and extends the selection
/// to newly revealed lines. Stops scheduling wakes immediately once the pointer re-enters the
/// viewport, reaches a scroll boundary, or the drag ends.
pub(crate) fn poll_gutter_drag(st: &mut CodeState, tx: &mut Tx) {
    let Some(mut drag) = st.gutter_drag() else {
        return;
    };
    let speed = edge_speed(st, drag.pointer_y);
    if let Some(row_idx) = st.position.drag_row(drag.pointer_y) {
        let target_line = st.rows().row(row_idx).line;
        st.apply_gutter_lines(drag.anchor_line, target_line, tx);
    }
    let Some(speed) = speed else {
        drag.last_tick = None;
        st.set_gutter_drag(Some(drag));
        return;
    };
    let now = tx.clock();
    let dt = drag
        .last_tick
        .map_or(0.0, |prev| (now - prev).clamp(0.0, EDGE_MAX_STEP_SECS));
    drag.last_tick = Some(now);
    st.set_gutter_drag(Some(drag));
    if dt > 0.0 {
        let p = st.position.scroll_pos();
        let next_y = (p.offset[1] + speed * dt as f32).clamp(0.0, p.max[1]);
        if (next_y - p.offset[1]).abs() > 0.01 {
            tx.scroll_to_instant(st.scroll_key(), p.offset[0], next_y);
        }
    }
    tx.wake_after(Duration::ZERO);
}

/// Signed auto-scroll velocity in points per second when `y` lies outside the viewport and
/// room remains to scroll in that direction.
pub(crate) fn edge_speed(st: &CodeState, y: f32) -> Option<f32> {
    let (vp_top, vp_h) = st.position.viewport_bounds();
    let p = st.position.scroll_pos();
    if vp_h <= 0.0 || p.max[1] <= 0.5 {
        return None;
    }
    let vp_bottom = vp_top + vp_h;
    if y < vp_top && p.offset[1] > 0.5 {
        let past = (vp_top - y).max(EDGE_MIN_PAST);
        Some(-(past * EDGE_RATE).min(EDGE_MAX_SPEED))
    } else if y > vp_bottom && p.offset[1] < p.max[1] - 0.5 {
        let past = (y - vp_bottom).max(EDGE_MIN_PAST);
        Some((past * EDGE_RATE).min(EDGE_MAX_SPEED))
    } else {
        None
    }
}
