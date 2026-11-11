//! The document-position scroll bar (`preview/preview-panel/documents/scrollbar`): a thumb
//! that follows where the reader is in the rows, not Rune's estimate of the content's height,
//! which moves as rows are measured. Strata's rule (`scrollbar/position.rs` in
//! `src/ui/virtual_preview`):
//!
//! - the position is the row at the viewport's top plus how far into it the top is, and the
//!   end the same at the bottom; progress is start over (rows - visible rows);
//! - at the first row's top it is 0 and with the last row's end in view 1;
//! - otherwise it never moves backward while the content moves forward, never forward while
//!   it moves back, and holds while the content holds, so a row measured below the viewport
//!   can't nudge the thumb.
//!
//! A drag moves the content to the row and fraction the thumb stands for, and either end of
//! the track to the exact first or last line. The rows report where they are drawn
//! (`on_layout_tx`, live only), so scrolling never runs the view for the bar.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use rune::{Color, Live, ScrollPos, Tx};

use crate::scroll::BarLook;
use crate::scroll::bar::thumb;

/// The geometry the view lays out with, set on every build.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Keys {
    pub scroll: String,
    pub list: String,
    pub row_prefix: String,
    /// Rows have one fixed height (wrap off): offsets map to rows exactly.
    pub fixed: Option<f32>,
}

struct Inner {
    keys: RefCell<Keys>,
    rows: Cell<usize>,
    visible: Cell<(usize, usize)>,
    /// Where mounted rows are drawn, in window points: `(top, height)`.
    rects: RefCell<HashMap<usize, (f32, f32)>>,
    /// The viewport's top and height in window points.
    viewport: Cell<(f32, f32)>,
    pos: Cell<ScrollPos>,
    /// The last published `(start, progress)`, and the visible rows then.
    published: Cell<Option<(f64, f64)>>,
    span: Cell<f64>,
    /// The track's length: the bar's height.
    track: Cell<f32>,
    look: Cell<Option<BarLook>>,
    thumb: Live<[f32; 4]>,
    fill: Live<Color>,
    over: Cell<bool>,
    dragging: Cell<bool>,
    grab: Cell<f32>,
    /// A drag's target, as a row position, waiting for its row to mount.
    pending: Cell<Option<f64>>,
    /// The row the bar's accessible value last said: steps from assistive technology count
    /// from it.
    said: Cell<f64>,
    /// The maximum the bar's accessible range last said.
    said_max: Cell<f64>,
    /// A step from the keyboard or assistive technology moved the content: the bar's value
    /// should say where it went.
    stepped: Cell<bool>,
}

impl Default for Inner {
    fn default() -> Self {
        Inner {
            keys: RefCell::default(),
            rows: Cell::new(0),
            visible: Cell::new((0, 0)),
            rects: RefCell::default(),
            viewport: Cell::new((0.0, 0.0)),
            pos: Cell::new(ScrollPos::default()),
            published: Cell::new(None),
            span: Cell::new(0.0),
            track: Cell::new(0.0),
            look: Cell::new(None),
            thumb: Live::new([0.0; 4]),
            fill: Live::new(Color::TRANSPARENT),
            over: Cell::new(false),
            dragging: Cell::new(false),
            grab: Cell::new(0.0),
            pending: Cell::new(None),
            said: Cell::new(0.0),
            said_max: Cell::new(0.0),
            stepped: Cell::new(false),
        }
    }
}

/// The bar's state. The host keeps it in [`CodeState`](super::CodeState); clones share it.
#[derive(Clone, Default)]
pub(crate) struct Position(Rc<Inner>);

impl std::fmt::Debug for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Position")
            .field("start", &self.start())
            .field("progress", &self.progress())
            .finish()
    }
}

impl Position {
    /// A new text: forget what the old one measured, and any drag.
    pub fn reset(&self) {
        let i = &self.0;
        i.rects.borrow_mut().clear();
        i.published.set(None);
        i.pending.set(None);
        i.dragging.set(false);
        i.over.set(false);
    }

    /// Takes the view's keys, row count and look on each build.
    pub fn configure(&self, keys: Keys, rows: usize, look: BarLook) {
        let i = &self.0;
        if *i.keys.borrow() != keys {
            *i.keys.borrow_mut() = keys;
            i.published.set(None);
        }
        i.rows.set(rows);
        if i.look.replace(Some(look)) != Some(look) {
            i.fill.snap(if self.held() {
                look.trough_hover
            } else {
                look.trough
            });
        }
        self.layout();
    }

    pub fn set_visible(&self, rows: Range<usize>) {
        self.0.visible.set((rows.start, rows.end));
        // Rows far outside the window aren't mounted: their rects are stale.
        let n = rows.len().max(1);
        let keep = rows.start.saturating_sub(3 * n)..rows.end + 3 * n;
        self.0.rects.borrow_mut().retain(|r, _| keep.contains(r));
    }

    pub fn follow(&self, p: ScrollPos) {
        let prev = self.0.pos.replace(p);
        let dir = p.offset[1].total_cmp(&prev.offset[1]);
        self.publish(dir);
    }

    pub fn viewport(&self, rect: [f32; 4]) {
        self.0.viewport.set((rect[1], rect[3]));
        self.publish(Ordering::Equal);
    }

    pub fn track(&self, length: f32) {
        self.0.track.set(length);
        self.layout();
    }

    /// Row `row` is drawn at `rect`. Takes a drag's pending target once its row is mounted.
    pub fn row(&self, row: usize, rect: [f32; 4], tx: &mut Tx) {
        self.0.rects.borrow_mut().insert(row, (rect[1], rect[3]));
        if let Some(at) = self.0.pending.get()
            && at.floor() as usize == row
        {
            self.0.pending.set(None);
            self.place(at, tx);
        }
        self.publish(Ordering::Equal);
    }

    /// Whether a step moved the content since the bar last said its value, so the view should
    /// run to say the new one.
    pub fn moved_by_step(&self) -> bool {
        let i = &self.0;
        if i.stepped.get() && self.start().round() != i.said.get() {
            i.stepped.set(false);
            return true;
        }
        false
    }

    /// The viewport's vertical offset, as last reported.
    pub fn offset(&self) -> f32 {
        self.0.pos.get().offset[1]
    }

    /// Whether the text overflows, so the bar shows.
    pub fn overflows(&self) -> bool {
        self.0.pos.get().max[1] > 0.5
    }

    pub fn progress(&self) -> f64 {
        self.0.published.get().map_or(0.0, |p| p.1)
    }

    /// The row position at the viewport's top.
    pub fn start(&self) -> f64 {
        self.0.published.get().map_or(0.0, |p| p.0)
    }

    /// How many rows the viewport shows.
    pub fn span(&self) -> f64 {
        self.0.span.get()
    }

    pub fn thumb_live(&self) -> &Live<[f32; 4]> {
        &self.0.thumb
    }

    pub fn fill_live(&self) -> &Live<Color> {
        &self.0.fill
    }

    fn held(&self) -> bool {
        self.0.over.get() || self.0.dragging.get()
    }

    /// The row position drawn at window `y`, from a mounted row's rect.
    fn at(&self, y: f32) -> Option<f64> {
        let i = &self.0;
        if let Some(h) = i.keys.borrow().fixed {
            let (top, _) = i.viewport.get();
            let off = i.pos.get().offset[1];
            return Some(f64::from((off + y - top) / h.max(1.0)));
        }
        let (a, b) = i.visible.get();
        let rects = i.rects.borrow();
        let lo = a.saturating_sub(1);
        (lo..b + 1).find_map(|r| {
            let &(top, h) = rects.get(&r)?;
            (y >= top && y < top + h).then(|| r as f64 + f64::from((y - top) / h.max(1.0)))
        })
    }

    /// Works out the position from the drawn rows and lays the thumb out.
    fn publish(&self, dir: Ordering) {
        let i = &self.0;
        let n = i.rows.get() as f64;
        let p = i.pos.get();
        let (top, height) = i.viewport.get();
        if n == 0.0 || height <= 0.0 {
            return;
        }
        let at_top = p.offset[1] <= 0.5;
        let at_end = p.offset[1] >= p.max[1] - 0.5;
        let start = if at_top { Some(0.0) } else { self.at(top) };
        let end = if at_end {
            Some(n)
        } else {
            self.at(top + height - 0.01)
        };
        let (Some(start), Some(end)) = (start, end) else {
            return;
        };
        let start = start.clamp(0.0, n);
        let end = end.clamp(start, n);
        let visible = end - start;
        let range = n - visible;
        let mut progress = if range > 0.0 { start / range } else { 0.0 };
        if at_top || start == 0.0 {
            progress = 0.0;
        } else if at_end || end >= n {
            progress = 1.0;
        } else if let Some((prev_start, prev)) = i.published.get() {
            let dir = match dir {
                Ordering::Equal => start.total_cmp(&prev_start),
                d => d,
            };
            progress = match dir {
                Ordering::Greater => progress.max(prev),
                Ordering::Less => progress.min(prev),
                Ordering::Equal => prev,
            };
        }
        i.published.set(Some((start, progress.clamp(0.0, 1.0))));
        i.span.set(visible);
        self.layout();
    }

    /// The thumb's start and length on the track.
    fn thumb_at(&self) -> Option<(f32, f32, f32)> {
        let i = &self.0;
        let look = i.look.get()?;
        let track = i.track.get() - 2.0 * look.ends;
        let n = i.rows.get() as f32;
        if track <= 0.0 || n <= 0.0 {
            return None;
        }
        let shown = self.span() as f32;
        // The thumb's length is the share of rows shown; `thumb` takes it as a viewport over
        // the rest.
        let (_, len) = thumb(0.0, (n - shown).max(0.0), shown, track, look.min_length);
        let start = (track - len) * self.progress() as f32;
        Some((start + look.ends, len, track))
    }

    fn layout(&self) {
        let i = &self.0;
        let Some(look) = i.look.get() else { return };
        let Some((start, len, _)) = self.thumb_at() else {
            i.thumb.snap([0.0; 4]);
            return;
        };
        let inset = (look.thickness - look.thumb_width) / 2.0;
        i.thumb.snap([inset, start, look.thumb_width, len]);
    }

    /// Ends a drag the bar's element can no longer end: it was removed, so no Up comes. The
    /// trough goes back to rest at once.
    pub fn end_drag(&self) {
        let i = &self.0;
        if i.dragging.replace(false) | i.over.replace(false) {
            i.pending.set(None);
            if let Some(look) = i.look.get() {
                i.fill.snap(look.trough);
            }
        }
    }

    pub fn hover(&self, inside: bool) {
        self.0.over.set(inside);
        self.tint();
    }

    fn tint(&self) {
        if let Some(look) = self.0.look.get() {
            self.0.fill.set(if self.held() {
                look.trough_hover
            } else {
                look.trough
            });
        }
    }

    /// A drag on the bar at `y` from its top: a press on the thumb holds it where it was
    /// caught; one on the trough puts the thumb's middle there (GTK's warp), and the drag goes
    /// on from it.
    pub fn drag(&self, phase: rune::Phase, y: f32, tx: &mut Tx) {
        let i = &self.0;
        if phase == rune::Phase::Up {
            i.dragging.set(false);
            self.tint();
            return;
        }
        let Some((start, len, track)) = self.thumb_at() else {
            return;
        };
        if !self.overflows() {
            return;
        }
        if phase == rune::Phase::Down {
            i.dragging.set(true);
            self.tint();
            let on_thumb = (start..=start + len).contains(&y);
            i.grab.set(if on_thumb { y - start } else { len / 2.0 });
            if on_thumb {
                return;
            }
        }
        let ends = i.look.get().map_or(0.0, |l| l.ends);
        let travel = (track - len).max(1.0);
        let progress = f64::from(((y - i.grab.get() - ends) / travel).clamp(0.0, 1.0));
        let range = (i.rows.get() as f64 - self.span()).max(0.0);
        self.go(progress * range, tx);
    }

    /// Moves the content so row position `at` sits at the viewport's top: the ends exactly,
    /// a mounted row by its own place (`Tx::scroll_key_to`, from the layout this commit
    /// makes), any other once Rune has mounted it.
    pub fn go(&self, at: f64, tx: &mut Tx) {
        let i = &self.0;
        let range = (i.rows.get() as f64 - self.span()).max(0.0);
        let keys = i.keys.borrow().clone();
        if at <= 0.0 {
            i.pending.set(None);
            tx.scroll_to_y(keys.scroll, 0.0);
        } else if at >= range - 1e-6 || at >= i.said_max.get() - 0.5 && i.said_max.get() > 0.0 {
            i.pending.set(None);
            tx.scroll_to_bottom_instant(keys.scroll);
        } else if let Some(h) = keys.fixed {
            // Fixed rows: the offset is exact.
            i.pending.set(None);
            let x = i.pos.get().offset[0];
            tx.scroll_to_instant(keys.scroll, x, (at as f32) * h);
        } else {
            let r = at.floor() as usize;
            let (a, b) = i.visible.get();
            let span = (b - a).max(1);
            if r + 2 * span >= a && r <= b + 2 * span {
                i.pending.set(None);
                self.place(at, tx);
            } else {
                // Rune reveals the row (at the viewport's bottom, coming from above), then
                // it reports its place and lands on top. Coming from above, the reveal is
                // short of the target, so the content never moves back.
                tx.scroll_into_view_in(keys.list, format!("{}{r}", keys.row_prefix));
                i.pending.set(Some(at));
            }
        }
    }

    /// Puts row position `at`'s row at the viewport's top, `at`'s fraction of it above.
    fn place(&self, at: f64, tx: &mut Tx) {
        let i = &self.0;
        let keys = i.keys.borrow();
        let r = at.floor() as usize;
        let h = i.rects.borrow().get(&r).map_or(0.0, |&(_, h)| h);
        let above = (at - r as f64) as f32 * h;
        tx.scroll_key_to(format!("{}/{}{r}", keys.list, keys.row_prefix), -above);
    }

    /// The value the bar's last build gave assistive technology.
    pub fn said(&self, value: f64, max: f64) {
        self.0.said.set(value);
        self.0.said_max.set(max);
    }

    /// A step from assistive technology (or Left and Right): `delta` rows from the value it
    /// last read, so setting the minimum or maximum lands on the exact end.
    pub fn step_said(&self, delta: f64, tx: &mut Tx) {
        self.0.stepped.set(true);
        let range = (self.0.rows.get() as f64 - self.span()).max(0.0);
        let at = self.0.said.get() + delta;
        let end = at >= range || at >= self.0.said_max.get() - 0.5;
        self.go(if end { f64::INFINITY } else { at.max(0.0) }, tx);
    }

    /// A step from the keyboard: `delta` rows, clamped.
    pub fn step(&self, delta: f64, tx: &mut Tx) {
        self.0.stepped.set(true);
        let range = (self.0.rows.get() as f64 - self.span()).max(0.0);
        let at = (self.start().round() + delta).clamp(0.0, range);
        self.go(at, tx);
    }
}
