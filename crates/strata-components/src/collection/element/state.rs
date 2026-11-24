// SPDX-License-Identifier: MIT

//! [`CollectionState`]: what the host keeps for one collection, behind the element's accessor.

use std::cell::Cell;
use std::ops::Range;

use rune::{Live, Tx};

use super::keys::{line_path, viewport_path};
use super::lines::{Groups, LineMap};
use super::seam::{Controllers, Hit};
use crate::collection::selection::{Changed, Listing, Position, ScrollMemo, Selection, SyncRule};
use crate::contract::{EntryId, EntrySource, Point, Rect};

/// Where a drag hovering the collection would drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropOver {
    /// On this folder-like entry's row.
    Row(EntryId),
    /// On the collection's background: the folder it shows.
    Background,
}

/// One collection's state: its selection, its controllers.
pub struct CollectionState {
    /// The selection, cursor and anchor.
    pub selection: Selection,
    /// The controllers input is routed to.
    pub controllers: Controllers,
    pub(super) viewport: Rect,
    pub(super) scroll: Live<f32>,
    /// The scroll offset as a layer offset, `[0, -offset]`: what the marquee's layer rides.
    pub(super) scroll_layer: Live<[f32; 2]>,
    pub(super) visible: Range<usize>,
    pub(super) drop: Option<DropOver>,
    pub(super) drop_refused: Option<DropOver>,
    pub(super) drop_refused_files: bool,
    pub(super) hover_live: bool,
    /// The entry under the pointer at its last move over the collection (the rest hook's).
    pub(super) rest_under: Option<EntryId>,
    pub(super) rest_entered: Option<EntryId>,
    pub(super) keyboard: bool,
    pub(super) active: bool,
    pub(super) layout: u64,
    pub(super) pending: Option<Position>,
    pub(super) pending_offset: f32,
    pub(super) per_line: Cell<usize>,
    /// The row height, as last laid out.
    pub(super) row_height: f32,
    pub(super) gesture: Option<Hit>,
    /// The selection as last announced: its revision, how many were selected and of how many.
    pub(super) announced: (u64, usize, usize),
    /// A reveal asked for before the viewport had a width, done at its first layout.
    pub(super) reveal_after_layout: Cell<Option<EntryId>>,
    /// The groups the host draws the entries in ([`CollectionState::set_groups`]).
    pub(super) groups: Groups,
    /// The group whose header the last build stuck at the top, if any.
    pub(super) stuck_shown: Cell<Option<usize>>,
    /// The viewport has had a width: a later width that changes the count is a reflow.
    pub(super) laid_out: bool,
    /// The stuck header's offset, `[0, dy]`: up by as much as the next header pushes it.
    pub(super) stuck_offset: Live<[f32; 2]>,
    pub(super) top_line: Cell<Option<usize>>,
    pub(super) header: Cell<Rect>,
    pub(super) deferred: Option<super::seam::PointerInput>,
    pub(crate) origin_press: Option<(Point, rune::Mods)>,
}

impl Default for CollectionState {
    fn default() -> Self {
        Self {
            selection: Selection::new(),
            controllers: Controllers::default(),
            viewport: Rect::default(),
            scroll: Live::new(0.0),
            scroll_layer: Live::new([0.0, 0.0]),
            visible: 0..0,
            drop: None,
            drop_refused: None,
            drop_refused_files: false,
            hover_live: true,
            rest_under: None,
            rest_entered: None,
            keyboard: false,
            active: true,
            layout: 0,
            pending: None,
            pending_offset: 0.0,
            per_line: Cell::new(1),
            row_height: 0.0,
            gesture: None,
            announced: (0, 0, 0),
            reveal_after_layout: Cell::new(None),
            groups: Groups::default(),
            stuck_shown: Cell::new(None),
            laid_out: false,
            stuck_offset: Live::new([0.0, 0.0]),
            top_line: Cell::new(None),
            header: Cell::new(Rect::default()),
            deferred: None,
            origin_press: None,
        }
    }
}

impl std::fmt::Debug for CollectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectionState")
            .field("selection", &self.selection)
            .field("controllers", &self.controllers)
            .field("viewport", &self.viewport)
            .field("scroll", &self.scroll.get())
            .field("visible", &self.visible)
            .field("drop", &self.drop)
            .field("keyboard", &self.keyboard)
            .field("layout", &self.layout)
            .field("pending", &self.pending.is_some())
            .finish()
    }
}

impl CollectionState {
    /// Nothing selected, nothing recorded yet.
    pub fn new() -> Self {
        Self::default()
    }

    // Reading what the element recorded.

    /// The viewport's rect in window points, as last laid out.
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// How far the viewport is scrolled, in points, as last drawn.
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.get()
    }

    /// The scroll offset as a live value, which follows a scroll in the same frame with no view commit.
    pub fn scroll_live(&self) -> Live<f32> {
        self.scroll.clone()
    }

    /// The entries in the viewport, as last reported: what thumbnails load first.
    pub fn visible_range(&self) -> Range<usize> {
        self.visible.clone()
    }

    /// Whether any of the line holding entry `index` of `len` is in the viewport.
    pub fn shows_entry(&self, index: usize, len: usize) -> bool {
        let h = self.row_height;
        if h <= 0.0 || index >= len {
            return false;
        }
        let top = self.line_map(len).line_of(index) as f32 * h;
        let offset = self.scroll.get();
        top < offset + self.viewport.h && top + h > offset
    }

    /// The part of the content the viewport shows, in content coordinates.
    pub fn visible_content(&self) -> Rect {
        Rect::new(0.0, self.scroll.get(), self.viewport.w, self.viewport.h)
    }

    /// The content point under window point `window`.
    pub fn content_point(&self, window: Point) -> Point {
        Point::new(
            window.x - self.viewport.x,
            window.y - self.viewport.y + self.scroll.get(),
        )
    }

    /// `rect`, in content coordinates, as window points: where a popup anchored to an entry opens.
    pub fn content_to_window(&self, rect: Rect) -> Rect {
        rect.offset(self.viewport.x, self.viewport.y - self.scroll.get())
    }

    /// Where a drag hovering the collection would drop, while one does.
    pub fn drop_over(&self) -> Option<DropOver> {
        self.drop
    }

    /// Whether keyboard mode is on: the last input that moved the cursor was a key.
    pub fn keyboard_mode(&self) -> bool {
        self.keyboard
    }

    /// Whether rows draw hover now: off after keyboard navigation until the pointer moves.
    pub fn row_hover(&self) -> bool {
        self.hover_live
    }

    /// How many entries each line holds, as the last build or layout worked it out: 1 for a list, the tiles a row for a grid.
    pub fn per_line(&self) -> usize {
        self.per_line.get()
    }

    /// The layout key the scroll offset is remembered under: see [`ScrollMemo`].
    pub fn layout_key(&self) -> u64 {
        self.layout
    }

    pub(super) fn announcement(&self) -> super::Announcement {
        super::Announcement {
            selected: self.announced.1,
            total: self.announced.2,
        }
    }

    /// Whether a restore waits for the listing to complete.
    pub fn restore_pending(&self) -> bool {
        self.pending.is_some()
    }

    // Setting what the host decides.

    /// Sets the layout key: the view mode and anything that changes wrapping (an icons grid's width).
    pub fn set_layout_key(&mut self, layout: u64) {
        if layout != self.layout {
            self.pending = None;
        }
        self.layout = layout;
    }

    /// Sets where a hovering drag would drop, for a binder that hears hovers itself.
    pub fn set_drop_over(&mut self, drop: Option<DropOver>) {
        self.drop = drop;
    }

    /// Sets the groups the entries draw in, each under a header line, the top group's header stuck at the top of the viewport.
    pub fn set_groups(&mut self, groups: Groups) {
        self.groups = groups;
    }

    /// The groups the entries draw in: see [`CollectionState::set_groups`].
    pub fn groups(&self) -> &Groups {
        &self.groups
    }

    /// Shows the cursor or not: in Columns, only the column holding the keyboard cursor shows it.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Holds rest reports (`Collection::rest`) until the pointer next arrives on a row.
    pub fn hold_rest(&mut self) {
        self.rest_entered = None;
    }

    /// Records keyboard navigation that moved the cursor from outside the element's own key handling (a page shortcut).
    pub fn keyboard_moved(&mut self) {
        self.keyboard = true;
        self.hover_live = false;
    }

    /// Takes the selection's current state as the one the live status line reads out.
    pub fn note_selection(&mut self, source: &dyn EntrySource) {
        let revision = self.selection.revision();
        if revision != self.announced.0 {
            self.announced = (revision, self.selection.len(), source.len());
        }
    }

    // Keeping in step with the source.

    /// [`Selection::before_removal`]: call it right before the host removes entries.
    pub fn before_removal(&mut self, before: &dyn EntrySource, gone: impl Fn(EntryId) -> bool) {
        self.selection.before_removal(before, gone);
    }

    /// Brings the collection in step with `source` after it changed.
    pub fn sync(
        &mut self,
        scope: &str,
        source: &dyn EntrySource,
        rule: SyncRule,
        tx: &mut Tx,
    ) -> Changed {
        if rule.listing == Listing::Complete
            && let Some(position) = self.pending.take()
        {
            let changed = self.apply_position(scope, source, &position, rule, tx);
            self.note_unless_banding(source);
            return changed;
        }
        let changed = self.selection.sync(source, rule);
        if let Some(index) = self.selection.cursor_index(source) {
            tx.keep_on_screen(line_path(scope, source, index, self.per_line.get()));
        }
        self.note_unless_banding(source);
        changed
    }

    fn note_unless_banding(&mut self, source: &dyn EntrySource) {
        let banding = self
            .controllers
            .marquee
            .as_ref()
            .is_some_and(|m| m.active());
        if !banding {
            self.note_selection(source);
        }
    }

    /// How the last build laid out `len` entries.
    pub(super) fn line_map(&self, len: usize) -> LineMap {
        LineMap::new(self.per_line.get(), Some(&self.groups), len)
    }

    fn shows_line(&self, index: usize, len: usize) -> bool {
        let h = self.row_height;
        if h <= 0.0 {
            return false;
        }
        let map = self.line_map(len);
        let top = map.line_of(index) as f32 * h;
        let offset = self.scroll.get();
        let covered = if map.groups().is_some() { h } else { 0.0 };
        top >= offset + covered && top + h <= offset + self.viewport.h
    }

    pub(super) fn reflow_to(
        &self,
        scope: &str,
        source: &dyn EntrySource,
        per: usize,
        tx: &mut Tx,
    ) -> bool {
        let per = per.max(1);
        if per == self.per_line.get() {
            return false;
        }
        self.per_line.set(per);
        if let Some(index) = self.selection.cursor_index(source) {
            self.reveal(scope, source, index, tx);
        }
        true
    }

    /// Scrolls the entry at `index` into view, mounted or not, just far enough.
    pub fn reveal(&self, scope: &str, source: &dyn EntrySource, index: usize, tx: &mut Tx) {
        if index >= source.len() {
            return;
        }
        if self.viewport.w <= 0.0 {
            self.reveal_after_layout.set(Some(source.id_at(index)));
            return;
        }
        let map = self.line_map(source.len());
        if let Some(above) = map.key_above(source, index) {
            tx.scroll_into_view_in(scope.to_owned(), above);
        }
        tx.scroll_into_view_in(scope.to_owned(), map.line_key(source, map.line_of(index)));
    }

    pub(super) fn reveal_instant(
        &self,
        scope: &str,
        source: &dyn EntrySource,
        index: usize,
        tx: &mut Tx,
    ) {
        if index >= source.len() || self.row_height <= 0.0 || self.viewport.w <= 0.0 {
            return self.reveal(scope, source, index, tx);
        }
        let target = self
            .reveal_offset(source.len(), index)
            .unwrap_or_else(|| self.scroll.get());
        tx.scroll_to_instant(viewport_path(scope), 0.0, target);
    }

    fn reveal_offset(&self, len: usize, index: usize) -> Option<f32> {
        let h = self.row_height;
        let map = self.line_map(len);
        let top = map.line_of(index) as f32 * h;
        let offset = self.scroll.get();
        let covered = if map.groups().is_some() { h } else { 0.0 };
        if top < offset + covered {
            Some((top - covered).max(0.0))
        } else if top + h > offset + self.viewport.h {
            Some(top + h - self.viewport.h)
        } else {
            None
        }
    }

    // Position restore.

    /// The position to remember for this folder as it is left: the selection's, with the scroll offset in the current layout.
    pub fn position(&self, source: &dyn EntrySource) -> Position {
        self.selection
            .position(source)
            .with_scroll(ScrollMemo::new(self.layout, self.scroll.get()))
    }

    /// Shows a folder: `source` is its listing as it stands (`listing` says whether it is complete).
    pub fn enter(
        &mut self,
        scope: &str,
        source: &dyn EntrySource,
        position: Option<&Position>,
        listing: Listing,
        tx: &mut Tx,
    ) -> Changed {
        self.pending = None;
        self.drop = None;
        self.reveal_after_layout.set(None);
        self.top_line.set(None);
        // The folder left takes its gestures and keyboard mode with it.
        self.controllers.reset();
        self.gesture = None;
        self.deferred = None;
        self.origin_press = None;
        self.keyboard = false;
        self.hover_live = true;
        let changed = match position {
            Some(position) if listing == Listing::Loading => {
                self.pending = Some(position.clone());
                self.pending_offset = 0.0;
                self.scroll.set(0.0);
                let changed = self.selection.restore(&Position::default());
                tx.scroll_to_instant(viewport_path(scope), 0.0, 0.0);
                changed | self.selection.sync(source, SyncRule::loading())
            }
            Some(position) => self.apply_position(scope, source, position, SyncRule::default(), tx),
            None => {
                self.scroll.set(0.0);
                let changed = self.selection.restore(&Position::default());
                tx.scroll_to_instant(viewport_path(scope), 0.0, 0.0);
                let rule = match listing {
                    Listing::Complete => SyncRule::default(),
                    Listing::Loading => SyncRule::loading(),
                };
                changed | self.selection.sync(source, rule)
            }
        };
        self.note_selection(source);
        changed
    }

    /// Drops a restore that waits for its listing.
    pub fn cancel_restore(&mut self) {
        self.pending = None;
    }

    /// Ends the pointer's gesture: the press held, the click run and a rename a click left pending.
    pub fn end_gestures(&mut self) {
        self.gesture = None;
        self.origin_press = None;
        if let Some(pointer) = self.controllers.pointer.as_mut() {
            pointer.reset();
        }
    }

    fn apply_position(
        &mut self,
        scope: &str,
        source: &dyn EntrySource,
        position: &Position,
        rule: SyncRule,
        tx: &mut Tx,
    ) -> Changed {
        let changed = self.selection.restore(position) | self.selection.sync(source, rule);
        match position.scroll.and_then(|m| m.offset_for(self.layout)) {
            Some(offset) => tx.scroll_to_instant(viewport_path(scope), 0.0, offset),
            None => match self.selection.cursor_index(source) {
                Some(index) if self.shows_line(index, source.len()) => {
                    tx.stop_scroll(viewport_path(scope));
                }
                Some(index) => self.reveal(scope, source, index, tx),
                None => tx.scroll_to_instant(viewport_path(scope), 0.0, 0.0),
            },
        }
        changed
    }
}
