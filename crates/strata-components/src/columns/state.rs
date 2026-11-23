// SPDX-License-Identifier: MIT

use rune::{Live, Spring, Tx};
use strata_kit::look::Look;

use super::reveal::ColumnSpan;
use super::sources::ColumnSources;
use super::width::{self, Resize};
use super::{drag_scroll::DragScroll, keys};
use crate::collection::element::{CollectionState, viewport_path};
use crate::collection::keyboard::{KeyHandle, Keyboard};
use crate::collection::marquee::Marquee;
use crate::collection::pointer::PointerIntent;
use crate::collection::selection::{Listing, Position, SyncRule};
use crate::contract::{EntryId, EntrySource, Point, Rect};
use crate::scroll::{Autoscroll, AutoscrollPointer, Bar, ScrollFeed};

/// Where the keyboard cursor goes when a column opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// The active column stays where it is: mirroring's child column (Up or Down onto a folder), shown without moving focus.
    #[default]
    No,
    /// The new column becomes the active one, with nothing selected in it: a click on a folder, a double click.
    Column,
    /// The new column becomes the active one, with the cursor on its first entry once it has one: Right or Return on a folder.
    FirstEntry,
}

/// A folder a column opens on: its listing as it stands, whether that is complete, and the position remembered for it.
#[derive(Clone, Copy)]
pub struct Opening<'a> {
    /// The folder's listing.
    pub source: &'a dyn EntrySource,
    /// Whether it is complete or still loading.
    pub listing: Listing,
    /// The cursor, selection and scroll remembered for it.
    pub position: Option<&'a Position>,
}

impl<'a> Opening<'a> {
    /// `source`, complete or `listing`, with no position remembered.
    pub fn new(source: &'a dyn EntrySource, listing: Listing) -> Self {
        Self {
            source,
            listing,
            position: None,
        }
    }

    /// The same opening, restoring `position`.
    pub fn position(mut self, position: &'a Position) -> Self {
        self.position = Some(position);
        self
    }
}

/// How a column comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entrance {
    /// Slides 24 points in from the left as it fades in: a new column.
    Slide,
    /// Fades in where the column it replaces stood: a sibling.
    Fade,
    /// No motion: a rebuilt strip.
    Still,
}

/// A cursor a column places once its listing allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pending {
    None,
    /// On the first entry ([`Focus::FirstEntry`] on a folder still loading).
    First,
    Open(EntryId),
}

/// One column: its collection's state, and what the strip keeps for it.
pub struct ColumnState {
    /// The column's collection: its selection, cursor and controllers, as `ColumnsState::push` installs them.
    pub collection: CollectionState,
    /// The collection's key serial: new with each folder the column shows.
    pub(super) serial: u64,
    /// The shell's key serial: kept by a sibling that takes the column's place.
    pub(super) slot: u64,
    pub(super) feed: ScrollFeed,
    pub(super) bar: Bar,
    pub(super) drawn: Live<[f32; 4]>,
    pub(super) width: f32,
    pub(super) limit: Option<f32>,
    pub(super) entrance: Entrance,
    pub(super) keys: KeyHandle,
    pub(super) pending: Pending,
    pub(super) peek_gate: std::rc::Rc<std::cell::Cell<super::pointer::PeekGate>>,
}

/// How a header press was classified on `Down`, for `on_header_click` on `Up`.
#[derive(Clone, Copy, Debug)]
pub(super) enum HeaderPeek {
    Suppress,
    Peek(usize),
    Normal(usize),
}

impl ColumnState {
    /// The column's key handle, shared with its keyboard controller: the 10xer range, and commands from outside the column.
    pub fn key_handle(&self) -> &KeyHandle {
        &self.keys
    }

    /// The width it was given (dragged, fitted, spread or saved), in base points, before the text size.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Whether this column's vertical scrollbar thumb is currently dragged.
    pub fn is_dragging(&self) -> bool {
        self.bar.is_dragging()
    }
}

impl std::fmt::Debug for ColumnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnState")
            .field("serial", &self.serial)
            .field("width", &self.width)
            .field("collection", &self.collection)
            .finish_non_exhaustive()
    }
}

/// One window's column strip: its columns, which one holds the keyboard cursor, the strip's scroll position and bar.
pub struct ColumnsState {
    pub(super) key: String,
    pub(super) columns: Vec<ColumnState>,
    pub(super) active: usize,
    pub(super) targeted: Option<usize>,
    pub(super) held: Option<usize>,
    serial: u64,
    pub(super) strip_feed: ScrollFeed,
    pub(super) strip_bar: Bar,
    /// The strip's viewport in window points, as last laid out.
    pub(super) strip: Rect,
    /// Where the last frame drew the strip's viewport (`[x, y, w, h]` in window points).
    pub(super) strip_drawn: Live<[f32; 4]>,
    /// A reveal asked for before the strip had a width.
    pub(super) reveal_after_layout: bool,
    default_width: f32,
    cap: Option<f32>,
    pub(super) drag: DragScroll,
    pub(super) resize: Option<Resize>,
    /// The column whose edge the pointer rests on, past the caption's delay.
    pub(super) edge_hover: Option<usize>,
    pub(super) edge_lit: Option<usize>,
    pub(super) edge_press: Option<(u64, f64)>,
    /// The text scale the widths were last drawn at.
    scale: Option<f32>,
    narrow: Narrow,
    pub(super) autoscroll: Autoscroll,
    pub(super) spare: CollectionState,
    pub(super) opened: Option<(usize, EntryId)>,
    pub(super) press_was_active: bool,
    bar_mode: crate::scroll::BarMode,
    /// The host's clock at the last tick.
    pub(super) now: f64,
    /// How many ticks the view has had: the frame a reveal's request goes out in.
    ticks: u64,
    /// The last reveal, worked out again while the strip's width changes soon after it.
    pub(super) recheck: Option<Recheck>,
    /// Last built clipped-column slices `(left_clipped, visible_width)` per column.
    clipped: Vec<Option<(bool, f32)>>,
    pub(super) strip_overflows: std::cell::Cell<bool>,
    pub(super) peek_seq: Option<(f64, Point)>,
    /// Classification of the held header press.
    pub(super) header_peek: Option<HeaderPeek>,
}

/// A recent reveal of the active column ([`ColumnsState::tick_reveal`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Recheck {
    /// When it was asked for, on the host's clock.
    since: f64,
    /// The strip's drawn width it was last worked out for.
    page: f32,
    /// Where it sent the strip.
    target: f32,
    /// The strip's offset at the last tick.
    offset: f32,
    /// The tick of the frame whose requests carried its last glide, if it asked for one.
    asked: Option<u64>,
}

impl std::fmt::Debug for ColumnsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnsState")
            .field("key", &self.key)
            .field("columns", &self.columns)
            .field("active", &self.active)
            .field("targeted", &self.targeted)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Narrow {
    Idle,
    /// Armed in tick `.0`; `.1` once an area came after.
    Armed(u64, bool),
}

/// The reveal's glide: about Strata's 220 ms `emphasized_deceleration`.
pub(super) const REVEAL: Spring = Spring::new(0.22, 0.0);

const RECHECK: f64 = 0.5;

impl ColumnsState {
    /// No columns yet.
    pub fn new(key: impl Into<String>, autoscroll: Autoscroll) -> Self {
        Self {
            key: key.into(),
            columns: Vec::new(),
            active: 0,
            targeted: None,
            held: None,
            serial: 0,
            strip_feed: ScrollFeed::new(),
            strip_bar: Bar::new(),
            strip: Rect::default(),
            strip_drawn: Live::new([0.0; 4]),
            reveal_after_layout: false,
            default_width: width::STANDARD,
            cap: None,
            drag: DragScroll::new(),
            resize: None,
            edge_hover: None,
            edge_lit: None,
            edge_press: None,
            scale: None,
            narrow: Narrow::Armed(0, false),
            autoscroll,
            spare: CollectionState::new(),
            opened: None,
            press_was_active: false,
            bar_mode: crate::scroll::BarMode::Overlay,
            now: 0.0,
            ticks: 0,
            recheck: None,
            clipped: Vec::new(),
            strip_overflows: std::cell::Cell::new(false),
            peek_seq: None,
            header_peek: None,
        }
    }

    // Reading.

    /// How many columns there are.
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Column `depth`.
    pub fn column(&self, depth: usize) -> Option<&ColumnState> {
        self.columns.get(depth)
    }

    /// Column `depth`, to change.
    pub fn column_mut(&mut self, depth: usize) -> Option<&mut ColumnState> {
        self.columns.get_mut(depth)
    }

    /// The column that holds the keyboard cursor.
    pub fn active(&self) -> usize {
        self.active
    }

    /// The column whose header shows the actions.
    pub fn targeted(&self) -> usize {
        self.held.or(self.targeted).unwrap_or(self.active)
    }

    /// The key of column `depth`'s collection: its rows' key paths and scroll requests start there (`viewport_path`.
    pub fn column_key(&self, depth: usize) -> Option<String> {
        self.columns
            .get(depth)
            .map(|c| keys::column(&self.key, c.serial))
    }

    /// The strip's scroll position, as last drawn: how far it is scrolled sideways.
    pub fn strip_offset(&self) -> f32 {
        self.strip_feed.get().offset[0]
    }

    /// The strip's viewport in window points, as last laid out.
    pub fn strip_rect(&self) -> Rect {
        self.strip
    }

    /// Whether the horizontal strip currently overflows its viewport.
    pub fn strip_overflows(&self) -> bool {
        self.strip_overflows.get()
    }

    /// When the strip's and the columns' scroll bars show.
    pub fn set_bar_mode(&mut self, mode: crate::scroll::BarMode) {
        self.strip_bar.set_mode(mode);
        for c in &self.columns {
            c.bar.set_mode(mode);
        }
        self.bar_mode = mode;
    }

    /// The window's autoscroll, which the host drives (`Autoscroll::drive`) and whose shield it mounts.
    pub fn autoscroll(&self) -> &Autoscroll {
        &self.autoscroll
    }

    /// The column a held file drag would drop into, as of the last frame.
    pub fn drop_column(&self) -> Option<usize> {
        self.drag.pointer().and(self.drag.hovered())
    }

    /// Whether the pointer is on the edge of the last column (the one that meets the preview) or resizing it.
    pub fn last_edge_lit(&self) -> bool {
        let last = self.columns.len().checked_sub(1);
        last.is_some() && (self.edge_lit == last || self.resize.as_ref().map(|r| r.depth) == last)
    }

    /// Whether a column edge is being dragged right now.
    pub fn is_resizing(&self) -> bool {
        self.resize.is_some()
    }

    /// Whether any column's vertical scrollbar thumb is currently dragged.
    pub fn is_dragging(&self) -> bool {
        self.columns.iter().any(|c| c.bar.is_dragging())
    }

    // Settings.

    /// The width new columns open at, in base points.
    pub fn set_default_width(&mut self, base: f32) {
        self.default_width = width::sane(base).unwrap_or(width::STANDARD);
    }

    /// The width a column may take at most, in points.
    pub fn set_width_cap(&mut self, cap: Option<f32>) {
        self.cap = cap.filter(|c| c.is_finite() && *c > 0.0);
        if let (Narrow::Armed(at, _), Some(_)) = (self.narrow, self.cap) {
            self.narrow = Narrow::Armed(at, true);
        }
    }

    pub(super) fn cap(&self) -> Option<f32> {
        self.cap
    }

    // The lifecycle.

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    fn new_column(&mut self, slot: Option<u64>, look: &Look, entrance: Entrance) -> ColumnState {
        let serial = self.next_serial();
        let slot = slot.unwrap_or(serial);
        let key = keys::column(&self.key, serial);
        let feed = ScrollFeed::new();
        let keys = KeyHandle::default();
        let peek_gate = std::rc::Rc::new(std::cell::Cell::new(super::pointer::PeekGate::Open));
        let mut collection = CollectionState::new();
        collection.controllers.pointer = Some(Box::new(super::pointer::ColumnPointer::new(
            Box::new(AutoscrollPointer::new(
                Some(Box::new(PointerIntent::new())),
                self.autoscroll.clone(),
                key,
                feed.clone(),
            )),
            peek_gate.clone(),
        )));
        collection.controllers.keyboard = Some(Box::new(Keyboard::new(keys.clone())));
        collection.controllers.marquee = Some(Box::new(Marquee::new()));
        collection.set_own_viewport_reveals(true);
        let scale = width::scale(look);
        // A saved width wider than the area opens narrowed to it; the setting stays.
        let limit = self
            .cap
            .filter(|&cap| width::drawn(self.default_width, scale) > cap);
        let bar = Bar::new();
        bar.set_mode(self.bar_mode);
        ColumnState {
            collection,
            serial,
            slot,
            feed,
            bar,
            drawn: Live::new([0.0; 4]),
            width: self.default_width,
            limit,
            entrance,
            keys,
            pending: Pending::None,
            peek_gate,
        }
    }

    /// Opens a column after the last on `open` (the folder's listing as it stands, whether it is complete.
    pub fn push(&mut self, open: Opening<'_>, focus: Focus, look: &Look, tx: &mut Tx) -> usize {
        let column = self.new_column(None, look, Entrance::Slide);
        self.columns.push(column);
        let depth = self.columns.len() - 1;
        self.enter(depth, open, focus, look, tx);
        depth
    }

    /// Opens a sibling in column `depth`'s place.
    pub fn swap_from(
        &mut self,
        depth: usize,
        open: Opening<'_>,
        focus: Focus,
        look: &Look,
        tx: &mut Tx,
    ) -> usize {
        if depth >= self.columns.len() {
            return self.push(open, focus, look, tx);
        }
        let (slot, drawn) = self
            .columns
            .get(depth)
            .map_or((None, [0.0; 4]), |c| (Some(c.slot), c.drawn.get()));
        self.drop_from(depth);
        let column = self.new_column(slot, look, Entrance::Fade);
        column.drawn.set(drawn);
        self.columns.truncate(depth);
        self.columns.push(column);
        self.enter(depth, open, focus, look, tx);
        depth
    }

    fn enter(&mut self, depth: usize, open: Opening<'_>, focus: Focus, look: &Look, tx: &mut Tx) {
        let key = keys::column(&self.key, self.columns[depth].serial);
        let column = &mut self.columns[depth];
        column
            .collection
            .enter(&key, open.source, open.position, open.listing, tx);
        match focus {
            Focus::No => {
                let active = self.active.min(self.columns.len() - 1);
                self.set_active_quiet(active);
            }
            Focus::Column => {
                self.set_active_quiet(depth);
                self.reveal(look, tx);
            }
            Focus::FirstEntry => {
                self.set_active_quiet(depth);
                self.place_first(depth, open.source, open.listing, tx);
                self.reveal(look, tx);
            }
        }
    }

    /// Closes the columns from `len` on: they narrow and fade out, unless animations are off.
    pub fn truncate(&mut self, len: usize, look: &Look, tx: &mut Tx) {
        let len = len.max(1);
        if len >= self.columns.len() {
            return;
        }
        self.drop_from(len);
        self.columns.truncate(len);
        let active = self.active.min(len - 1);
        self.set_active_quiet(active);
        self.reveal(look, tx);
    }

    /// Reveals the active column in the strip: called when the preview drawer beside it opens, resumes.
    pub fn reveal_active(&mut self, look: &Look, tx: &mut Tx) {
        self.reveal(look, tx);
        self.reveal_after_layout = true;
    }

    fn drop_from(&mut self, len: usize) {
        for c in self.columns.iter().skip(len) {
            self.autoscroll
                .stop_gone(Some(&keys::column(&self.key, c.serial)));
            c.bar.release();
        }
        if self.resize.as_ref().is_some_and(|r| r.depth >= len) {
            self.resize = None;
        }
        if self.edge_hover.is_some_and(|d| d >= len) {
            self.edge_hover = None;
        }
        if self.edge_lit.is_some_and(|d| d >= len) {
            self.edge_lit = None;
        }
        if self.held.is_some_and(|d| d >= len) {
            self.held = None;
        }
        if self.targeted.is_some_and(|d| d >= len) {
            self.targeted = None;
        }
        if self.opened.is_some_and(|(d, _)| d + 1 >= len) {
            self.opened = None;
        }
    }

    /// Replaces every column with fresh ones for `sources` (another location, a rebuilt strip, a switch back to Columns).
    pub fn replace(
        &mut self,
        sources: &dyn ColumnSources,
        active: usize,
        position: &dyn Fn(usize) -> Option<Position>,
        look: &Look,
        tx: &mut Tx,
    ) {
        self.drop_from(0);
        self.columns.clear();
        self.drag.reset();
        // Old columns go at once, new ones come in at once.
        tx.instant();
        for depth in 0..sources.len() {
            let column = self.new_column(None, look, Entrance::Still);
            self.columns.push(column);
            let key = keys::column(&self.key, self.columns[depth].serial);
            let listing = if sources.loading(depth) {
                Listing::Loading
            } else {
                Listing::Complete
            };
            let pos = position(depth);
            self.columns[depth].collection.enter(
                &key,
                sources.source(depth),
                pos.as_ref(),
                listing,
                tx,
            );
        }
        if !self.columns.is_empty() {
            self.set_active_quiet(active.min(self.columns.len() - 1));
            self.reveal(look, tx);
        }
        if look.animations && self.strip_overflows.get() {
            tx.wake_after(std::time::Duration::ZERO);
        }
    }

    /// Back, Forward or Up to the ancestor in column `depth`.
    pub fn return_to(
        &mut self,
        depth: usize,
        sources: &dyn ColumnSources,
        came_from: Option<EntryId>,
        look: &Look,
        tx: &mut Tx,
    ) {
        if depth >= self.columns.len() {
            return;
        }
        self.truncate(depth + 1, look, tx);
        self.set_active_quiet(depth);
        let key = keys::column(&self.key, self.columns[depth].serial);
        let source = sources.source(depth);
        let column = &mut self.columns[depth];
        if let Some(index) = came_from.and_then(|id| source.index_of(id)) {
            column.collection.selection.select_only(source, index);
            column.collection.note_selection(source);
            column.collection.reveal(&key, source, index, tx);
        }
        self.reveal(look, tx);
    }

    /// Brings column `depth` in step with its listing after it changed (`CollectionState::sync`).
    pub fn sync(&mut self, depth: usize, source: &dyn EntrySource, rule: SyncRule, tx: &mut Tx) {
        let Some(column) = self.columns.get_mut(depth) else {
            return;
        };
        let key = keys::column(&self.key, column.serial);
        column.collection.sync(&key, source, rule, tx);
        let complete = rule.listing == Listing::Complete;
        match column.pending {
            Pending::First if !source.is_empty() => {
                column.pending = Pending::None;
                place(column, &key, source, 0, tx);
            }
            Pending::Open(id) if complete => {
                column.pending = Pending::None;
                if column.collection.selection.cursor().is_none()
                    && let Some(index) = source.index_of(id)
                {
                    place(column, &key, source, index, tx);
                }
            }
            Pending::First if complete => column.pending = Pending::None,
            _ => {}
        }
    }

    // The active column.

    /// Makes column `depth` the one that holds the keyboard cursor, and reveals it.
    pub fn set_active(&mut self, depth: usize, look: &Look, tx: &mut Tx) {
        if depth < self.columns.len() {
            self.set_active_quiet(depth);
            self.reveal(look, tx);
        }
    }

    /// Holds column `depth`'s header actions while a menu opened from it is open.
    pub fn hold_target(&mut self, depth: Option<usize>) {
        self.held = depth.filter(|d| *d < self.columns.len());
    }

    pub(super) fn set_active_quiet(&mut self, depth: usize) {
        self.active = depth;
        self.targeted = Some(depth);
        for (i, c) in self.columns.iter_mut().enumerate() {
            c.collection.set_active(i == depth);
            if i != depth {
                c.pending = Pending::None;
            }
        }
    }

    /// The cursor on column `depth`'s first entry, or once it has one.
    pub(super) fn place_first(
        &mut self,
        depth: usize,
        source: &dyn EntrySource,
        listing: Listing,
        tx: &mut Tx,
    ) {
        let key = keys::column(&self.key, self.columns[depth].serial);
        let column = &mut self.columns[depth];
        if column.collection.selection.cursor().is_some() {
            return;
        }
        if source.is_empty() {
            if listing == Listing::Loading {
                column.pending = Pending::First;
            }
            return;
        }
        place(column, &key, source, 0, tx);
    }

    pub(super) fn place_open(&mut self, depth: usize, sources: &dyn ColumnSources, tx: &mut Tx) {
        let key = keys::column(&self.key, self.columns[depth].serial);
        let source = sources.source(depth);
        let open = sources.open_child(depth);
        let loading = sources.loading(depth);
        let column = &mut self.columns[depth];
        if column.collection.selection.cursor().is_some() {
            return;
        }
        match open.and_then(|id| source.index_of(id).map(|i| (id, i))) {
            Some((_, index)) => place(column, &key, source, index, tx),
            None => {
                if let (Some(id), true) = (open, loading) {
                    column.pending = Pending::Open(id);
                }
            }
        }
    }

    /// Clears the selection in every column (the cursors stay) and resets pending click gestures.
    pub fn clear_selection(&mut self, sources: &dyn ColumnSources) {
        for c in &mut self.columns {
            if let Some(ctl) = c.collection.controllers.pointer.as_mut() {
                ctl.reset();
            }
        }
        self.clear_all(sources);
    }

    pub(super) fn clear_before(&mut self, limit: usize, sources: &dyn ColumnSources) {
        for (depth, c) in self.columns.iter_mut().enumerate().take(limit) {
            if let Some(ctl) = c.collection.controllers.pointer.as_mut() {
                ctl.reset();
            }
            if depth < sources.len() && !c.collection.selection.is_empty() {
                c.collection.selection.clear();
                c.collection.note_selection(sources.source(depth));
            }
        }
    }

    /// Clears the selection in every column (the cursors stay).
    pub(super) fn clear_all(&mut self, sources: &dyn ColumnSources) {
        for (depth, c) in self.columns.iter_mut().enumerate() {
            if depth < sources.len() && !c.collection.selection.is_empty() {
                c.collection.selection.clear();
                c.collection.note_selection(sources.source(depth));
            }
        }
    }

    // The strip's scroll.

    /// Each column's span in strip content points, root first, as the widths stand.
    pub(super) fn spans(&self, look: &Look) -> Vec<(f32, f32)> {
        let scale = width::scale(look);
        let mut x = 0.0;
        self.columns
            .iter()
            .enumerate()
            .map(|(d, c)| {
                let w = self.drawn_width(d, c, scale);
                let span = (x, x + w);
                x += w;
                span
            })
            .collect()
    }

    /// Column `depth`'s width as drawn now: its own, or a resize's live one.
    pub(super) fn drawn_width(&self, depth: usize, c: &ColumnState, scale: f32) -> f32 {
        match &self.resize {
            Some(r) if r.depth == depth => width::drawn(r.live, scale),
            _ => {
                let w = width::drawn(c.width, scale);
                match c.limit {
                    Some(limit) => w.min(limit.max(width::drawn(width::STANDARD, scale))),
                    None => w,
                }
            }
        }
    }

    /// Every open column at `base` points (an edge's release spreads its width), none narrowed.
    pub(super) fn spread(&mut self, base: f32) {
        for c in &mut self.columns {
            c.width = base;
            c.limit = None;
        }
    }

    /// Column `depth` at `base` points, not narrowed (an autofit).
    pub(super) fn set_column_width(&mut self, depth: usize, base: f32) {
        if let Some(c) = self.columns.get_mut(depth) {
            c.width = base;
            c.limit = None;
        }
    }

    pub(super) fn tick_widths(&mut self, look: &Look, tx: &mut Tx) -> bool {
        let scale = width::scale(look);
        let mut changed = false;
        if self
            .scale
            .is_some_and(|was| (was - scale).abs() > f32::EPSILON)
        {
            for c in &mut self.columns {
                changed |= c.limit.take().is_some();
            }
            self.narrow = Narrow::Armed(self.ticks, false);
        }
        self.scale = Some(scale);
        let Narrow::Armed(at, arrived) = self.narrow else {
            return changed;
        };
        let Some(cap) = self.cap else {
            return changed;
        };
        if !arrived && self.ticks < at + 2 {
            return changed;
        }
        self.narrow = Narrow::Idle;
        let floor = width::drawn(width::STANDARD, scale);
        let mut narrowed = false;
        for c in &mut self.columns {
            if width::drawn(c.width, scale).min(c.limit.unwrap_or(f32::INFINITY)) > cap.max(floor) {
                c.limit = Some(cap);
                narrowed = true;
            }
        }
        if narrowed {
            tx.instant();
        }
        changed | narrowed
    }

    pub(super) fn tick_reveal(&mut self, now: f64, look: &Look, tx: &mut Tx) {
        self.now = now;
        self.ticks += 1;
        let Some(mut r) = self.recheck.take() else {
            return;
        };
        if now - r.since > RECHECK {
            return;
        }
        let pos = self.strip_feed.get();
        let (page, offset) = (pos.drawn[0], pos.offset[0]);
        let resized = (page - r.page).abs() > 0.5;
        // The view's glide moves the strip toward its target only, without overshooting.
        let (moved, ahead) = (offset - r.offset, r.target - r.offset);
        let foreign = moved.abs() > 0.5 && (ahead.abs() < 0.5 || moved.signum() != ahead.signum());
        if foreign {
            if r.asked.is_some_and(|t| t + 1 >= self.ticks) {
                tx.scroll_to_instant(keys::strip(&self.key), offset, 0.0);
            }
            return;
        }
        r.offset = offset;
        if resized {
            r.page = page;
            let (target, asked) = self.glide_to_active(look, tx);
            r.target = target;
            if asked {
                r.asked = Some(self.ticks);
            }
            tx.wake_after(std::time::Duration::ZERO);
        }
        self.recheck = Some(r);
    }

    pub(super) fn reveal(&mut self, look: &Look, tx: &mut Tx) {
        if self.strip.w <= 0.0 || self.columns.is_empty() {
            self.reveal_after_layout = true;
            return;
        }
        self.reveal_after_layout = false;
        let pos = self.strip_feed.get();
        let (target, asked) = self.glide_to_active(look, tx);
        self.recheck = Some(Recheck {
            since: self.now,
            page: pos.drawn[0],
            target,
            offset: pos.offset[0],
            // Handlers run before the frame's tick.
            asked: asked.then_some(self.ticks + 1),
        });
    }

    fn glide_to_active(&mut self, look: &Look, tx: &mut Tx) -> (f32, bool) {
        let page = self.strip.w;
        let spans = self.spans(look);
        let active = self.active.min(spans.len() - 1);
        let (left, right) = spans[active];
        let total = spans.last().map_or(0.0, |s| s.1);
        let span = ColumnSpan {
            left,
            right,
            trailing: total - right,
        };
        let current = self.strip_offset();
        let max = (total - page).max(0.0);
        let target = span.reveal_target(current, page, 0.0, max);
        if (target - current).abs() < 0.5 {
            return (current, false);
        }
        let strip = keys::strip(&self.key);
        if look.animations {
            tx.scroll_to_with(strip, target, 0.0, REVEAL);
        } else {
            tx.scroll_to_instant(strip, target, 0.0);
        }
        tx.wake_after(std::time::Duration::ZERO);
        (target, true)
    }

    pub(super) fn clipped_slice(&self, depth: usize, look: &Look) -> Option<(bool, f32)> {
        let page = self.strip.w;
        if page <= 0.0 || depth >= self.columns.len() {
            return None;
        }
        let spans = self.spans(look);
        let &(left, right) = spans.get(depth)?;
        let w = right - left;
        let x = left - self.strip_offset();
        let visible = ((x + w).min(page) - x.max(0.0)).max(0.0);
        let partial = w <= page + 1.0 && visible > 1.0 && visible < w - 1.0;
        partial.then_some((x < 0.0, visible.floor()))
    }

    pub(super) fn sync_clipped(&mut self, look: &Look) -> bool {
        let next: Vec<Option<(bool, f32)>> = (0..self.columns.len())
            .map(|d| self.clipped_slice(d, look))
            .collect();
        if next != self.clipped {
            self.clipped = next;
            true
        } else {
            false
        }
    }

    pub(super) fn strip_overflows_now(&self, look: &Look) -> bool {
        let bar = crate::scroll::BarLook::strip(look);
        let page = self.strip.w;
        if page < bar.min_length {
            return false;
        }
        let total = self.spans(look).last().map_or(0.0, |s| s.1);
        if total - page > 0.5 {
            return true;
        }
        let bar_pos = self.strip_bar.pos();
        let bar_fresh = (bar_pos.viewport[0] - page).abs() <= 0.5;
        look.animations && bar_fresh && self.strip_bar.overflows()
    }

    pub(super) fn sync_strip_overflow(&self, look: &Look) -> bool {
        let next = self.strip_overflows_now(look);
        if next != self.strip_overflows.get() {
            self.strip_overflows.set(next);
            true
        } else {
            false
        }
    }

    /// Consumes a second press within double-click time and distance of a peek-reveal click.
    pub(super) fn consume_peek_double_click(&mut self, now: f64, window: Point) -> bool {
        let Some((at, pt)) = self.peek_seq else {
            return false;
        };
        let within = (now - at) >= 0.0
            && (now - at) <= 0.4
            && (window.x - pt.x).abs() <= 5.0
            && (window.y - pt.y).abs() <= 5.0;
        if within {
            self.peek_seq = None;
            true
        } else {
            self.peek_seq = None;
            false
        }
    }

    // Drags.

    /// Where a file drag is over the strip (window points), accepted or refused, or that it left the strip.
    pub fn drag_over(&mut self, at: Option<Point>) -> bool {
        self.drag.over(at)
    }

    /// Ends a column-edge resize, a held drag or a scroll-bar drag.
    pub fn reset(&mut self) {
        self.resize = None;
        self.edge_hover = None;
        self.edge_lit = None;
        self.drag.reset();
        self.strip_bar.release();
        for c in &self.columns {
            c.bar.release();
        }
    }

    /// The scroll request target for column `depth`'s listing.
    pub(super) fn listing_path(&self, depth: usize) -> Option<String> {
        self.column_key(depth).map(|k| viewport_path(&k))
    }

    /// The column whose collection is keyed by `serial`, if it is still open.
    pub(super) fn depth_of(&self, serial: u64) -> Option<usize> {
        self.columns.iter().position(|c| c.serial == serial)
    }
}

fn place(column: &mut ColumnState, key: &str, source: &dyn EntrySource, index: usize, tx: &mut Tx) {
    column.collection.selection.select_only(source, index);
    column.collection.note_selection(source);
    column.collection.keyboard_moved();
    column.collection.reveal(key, source, index, tx);
}
