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

/// One collection's state: its selection, its controllers, and what the element records about
/// its viewport (where it shows, how far it is scrolled, which rows are visible).
///
/// The host keeps one per collection (a list, a grid, one Columns column) for the life of the
/// view, and hands it to the element by accessor. A folder change resets it with
/// [`CollectionState::enter`] rather than replacing it, so the scroll offset the element
/// follows stays the same value.
///
/// Used by: hosts, C2 (one per column), C3, C4, C14b, C18, C21.
pub struct CollectionState {
    /// The selection, cursor and anchor (C1a).
    pub selection: Selection,
    /// The controllers input is routed to (C1b, C1c, C1d).
    pub controllers: Controllers,
    pub(super) viewport: Rect,
    pub(super) scroll: Live<f32>,
    /// The scroll offset as a layer offset, `[0, -offset]`: what the marquee's layer rides.
    pub(super) scroll_layer: Live<[f32; 2]>,
    pub(super) visible: Range<usize>,
    pub(super) drop: Option<DropOver>,
    /// The zone (a folder row or the background) a drag is over that refuses it: its moves
    /// still reach the drag-over hook, but draw no drop highlight.
    pub(super) drop_refused: Option<DropOver>,
    /// Whether that refused drag offers files: only a file drag drives the hook (autoscroll
    /// follows file drags only).
    pub(super) drop_refused_files: bool,
    pub(super) hover_live: bool,
    /// The entry under the pointer at its last move over the collection (the rest hook's).
    pub(super) rest_under: Option<EntryId>,
    /// The entry the pointer last arrived on by moving, unless a press ended or the host held
    /// rest reports since: the only row whose rest report counts.
    pub(super) rest_entered: Option<EntryId>,
    pub(super) keyboard: bool,
    pub(super) active: bool,
    pub(super) layout: u64,
    pub(super) pending: Option<Position>,
    /// The scroll offset `enter` asked for while a restore waits: a scroll away from it is the
    /// person's, and cancels the restore.
    pub(super) pending_offset: f32,
    /// How many entries a line holds, as the last build or layout worked it out: a cell so the
    /// view (which reads the state) can record it, and sync, reveal and the visible range
    /// agree with what it built.
    pub(super) per_line: Cell<usize>,
    /// The row height, as last laid out.
    pub(super) row_height: f32,
    /// The pointer gesture in progress belongs to the collection (its press landed in the
    /// viewport), with what the press was over.
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
    /// The first line Rune last reported visible: the one whose group's header sticks. Rune
    /// reports it from the layout it is drawing, which the recorded scroll offset can trail.
    pub(super) top_line: Cell<Option<usize>>,
    /// The header part's rect in window points, as last laid out (`Collection::header`): a
    /// press on its blank space is on empty space, at the viewport's top. A cell, so a build
    /// with no header clears it.
    pub(super) header: Cell<Rect>,
    /// A press on the header's blank space held back until it travels past the threshold
    /// (`Collection::header_defers_press`).
    pub(super) deferred: Option<super::seam::PointerInput>,
    /// Reveals move only this collection's viewport
    /// ([`CollectionState::set_own_viewport_reveals`]).
    pub(super) own_viewport_reveals: bool,
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
            own_viewport_reveals: false,
            deferred: None,
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

    /// The scroll offset as a live value, which follows a scroll in the same frame with no view
    /// commit: an overlay drawn in viewport coordinates (C1d's band, C24's marker) rides it.
    pub fn scroll_live(&self) -> Live<f32> {
        self.scroll.clone()
    }

    /// The entries in the viewport, as last reported: what thumbnails load first (C25).
    ///
    /// Rune reports the rows a commit laid out on the frame after it, so for one frame after
    /// a reflow (a grid's width) or a regroup this is the range before it (Rune draft
    /// [U-C1f-5]). It is a priority hint, and catches up on its own.
    ///
    /// [U-C1f-5]: crate::collection::element#rune-drafts-not-filed
    pub fn visible_range(&self) -> Range<usize> {
        self.visible.clone()
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

    /// `rect`, in content coordinates, as window points: where a popup anchored to an entry
    /// opens (C21).
    pub fn content_to_window(&self, rect: Rect) -> Rect {
        rect.offset(self.viewport.x, self.viewport.y - self.scroll.get())
    }

    /// Where a drag hovering the collection would drop, while one does.
    pub fn drop_over(&self) -> Option<DropOver> {
        self.drop
    }

    /// Whether keyboard mode is on: the last input that moved the cursor was a key. The cursor
    /// outline shows only then, and row hover waits for the pointer to move.
    pub fn keyboard_mode(&self) -> bool {
        self.keyboard
    }

    /// Whether rows draw hover now: off after keyboard navigation until the pointer moves.
    pub fn row_hover(&self) -> bool {
        self.hover_live
    }

    /// How many entries each line holds, as the last build or layout worked it out: 1 for a
    /// list, the tiles a row for a grid (C3's count, for C25's `Viewport::per_line`).
    pub fn per_line(&self) -> usize {
        self.per_line.get()
    }

    /// The layout key the scroll offset is remembered under: see [`ScrollMemo`].
    pub fn layout_key(&self) -> u64 {
        self.layout
    }

    /// The text the live status line reads now: the last announced selection, worded by
    /// `wording`.
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

    /// Sets the layout key: the view mode and anything that changes wrapping (an icons grid's
    /// width). A remembered scroll offset comes back only in the layout it was measured in. A
    /// new layout (a view switch) cancels a restore that waits for its listing.
    pub fn set_layout_key(&mut self, layout: u64) {
        if layout != self.layout {
            self.pending = None;
        }
        self.layout = layout;
    }

    /// Sets where a hovering drag would drop, for a binder that hears hovers itself (X2's
    /// system shim). Rune's binder sets it through the rows' drop zones.
    pub fn set_drop_over(&mut self, drop: Option<DropOver>) {
        self.drop = drop;
    }

    /// Sets the groups the entries draw in, each under a header line, the top group's header
    /// stuck at the top of the viewport (C4's file-type groups; see [`Groups`]). Empty groups
    /// (the default) draw the entries ungrouped.
    ///
    /// The groups belong to the listing: set them whenever it changes, before
    /// [`CollectionState::enter`] or [`CollectionState::sync`] with it, so the restore and the
    /// reveal those make place the cursor among the new header lines. Groups that don't cover
    /// the listing (their lengths don't add up to its length) draw it ungrouped.
    pub fn set_groups(&mut self, groups: Groups) {
        self.groups = groups;
    }

    /// The groups the entries draw in: see [`CollectionState::set_groups`].
    pub fn groups(&self) -> &Groups {
        &self.groups
    }

    /// Shows the cursor or not: in Columns, only the column holding the keyboard cursor
    /// shows it (C2).
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Holds rest reports ([`Collection::rest`](super::Collection::rest)) until the pointer
    /// next arrives on a row: call it when a drag that didn't start in this collection ends
    /// (a sidebar row, a tab, another pane, a system drag from P1 or X2). A drag dropped on a
    /// folder then never peeks it. The collection does this itself when one of its own
    /// presses ends, and when a drop lands on one of its folder rows, which it also records as
    /// the row under the pointer (a system drag moves the pointer there unseen).
    pub fn hold_rest(&mut self) {
        self.rest_entered = None;
    }

    /// Records keyboard navigation that moved the cursor from outside the element's own key
    /// handling (a page shortcut): keyboard mode turns on, and row hover waits for the pointer
    /// to move.
    pub fn keyboard_moved(&mut self) {
        self.keyboard = true;
        self.hover_live = false;
    }

    /// Takes the selection's current state as the one the live status line reads out, if an
    /// operation changed it since the last time (its revision moved on). The element does this
    /// after every input and in [`CollectionState::sync`] and [`CollectionState::enter`]; a
    /// host that changes the selection itself (a Select All shortcut) calls it after.
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

    /// Brings the collection in step with `source` after it changed: [`Selection::sync`], then
    /// the cursor row keeps its place on screen (`Tx::keep_on_screen`). When the listing is
    /// complete and a restore waits for it, the restore applies now instead. While a marquee
    /// band is open, the status line waits for it to end.
    ///
    /// `scope` is the collection's key, as given to the element.
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
            // Rune holds a virtual row by its key: in a grid, the line the cursor's cell sits
            // in now. When the change moved the cell to another line, that line's key is new,
            // Rune finds no drawn rect to hold, and lets it go.
            tx.keep_on_screen(line_path(scope, source, index, self.per_line.get()));
        }
        self.note_unless_banding(source);
        changed
    }

    /// [`CollectionState::note_selection`], unless a marquee band is open: the status line
    /// reads a band's count once its gesture ends (the element notes it then), not a count a
    /// listing change makes mid-band.
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

    /// Whether the line holding entry `index` shows whole in the viewport, as last laid out,
    /// and clear of a stuck group header (which covers the viewport's top line).
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

    /// Takes `per` entries a line from now on. When that differs from before and there is a
    /// cursor, scrolls the cursor into view on `tx`: a reflow moves every entry to another
    /// line, and the lines' keys with them. True when the count changed.
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

    /// Keeps every reveal of an entry (a key, a placed cursor, a reflow, a restore) to this
    /// collection's own viewport, as one instant scroll worked out from the geometry (where
    /// [`CollectionState::reveal`] would land), rather than a reveal request: Rune applies a
    /// reveal request to the nearest sideways viewport around the collection too, at once
    /// (Rune draft U-C2-4). For a collection inside a sideways strip that scrolls itself to
    /// it, on a glide of its own (C2's columns). Off by default.
    pub fn set_own_viewport_reveals(&mut self, on: bool) {
        self.own_viewport_reveals = on;
    }

    /// Scrolls the entry at `index` into view, mounted or not, just far enough. In a grouped
    /// collection it lands clear of the stuck header: going up, the line above it (another
    /// entry, or its group's header) shows under the stuck header.
    pub fn reveal(&self, scope: &str, source: &dyn EntrySource, index: usize, tx: &mut Tx) {
        if index >= source.len() {
            return;
        }
        if self.viewport.w <= 0.0 || (self.own_viewport_reveals && self.row_height <= 0.0) {
            self.reveal_after_layout.set(Some(source.id_at(index)));
            return;
        }
        if self.own_viewport_reveals {
            return self.reveal_by_offset(scope, source, index, tx);
        }
        let map = self.line_map(source.len());
        // Rune runs a transaction's reveals in order, each from where the one before left the
        // view (`runtime/frame.rs`): the line above comes to the top when the entry is above,
        // and the entry to the bottom when it is below; neither moves a view showing both.
        if let Some(above) = map.key_above(source, index) {
            tx.scroll_into_view_in(scope.to_owned(), above);
        }
        tx.scroll_into_view_in(scope.to_owned(), map.line_key(source, map.line_of(index)));
    }

    /// [`CollectionState::reveal`] as one instant scroll worked out from the geometry, to the
    /// same place (going up, the line above at the top, under any stuck header; going down,
    /// the entry at the bottom; nowhere when it shows): for a view a glide is moving, which a
    /// reveal request would keep moving (C1d's S-B, a key during a band's edge scroll).
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
        self.reveal_by_offset(scope, source, index, tx);
    }

    /// The scroll of [`CollectionState::reveal_instant`], on a laid-out viewport: an offset
    /// request on this collection's viewport alone (Rune leaves a viewport's other axis and
    /// the viewports around it where they are).
    fn reveal_by_offset(&self, scope: &str, source: &dyn EntrySource, index: usize, tx: &mut Tx) {
        let h = self.row_height;
        let map = self.line_map(source.len());
        let line = map.line_of(index);
        let top = line as f32 * h;
        let offset = self.scroll.get();
        let covered = if map.groups().is_some() { h } else { 0.0 };
        let above = if line > 0 { h } else { 0.0 };
        let target = if top < offset + covered {
            (top - above - covered).max(0.0)
        } else if top + h > offset + self.viewport.h {
            top + h - self.viewport.h
        } else {
            offset
        };
        tx.scroll_to_instant(viewport_path(scope), 0.0, target);
    }

    // Position restore.

    /// The position to remember for this folder as it is left: the selection's, with the
    /// scroll offset in the current layout.
    pub fn position(&self, source: &dyn EntrySource) -> Position {
        self.selection
            .position(source)
            .with_scroll(ScrollMemo::new(self.layout, self.scroll.get()))
    }

    /// Shows a folder: `source` is its listing as it stands (`listing` says whether it is
    /// complete), and `position` what was remembered for it, if anything.
    ///
    /// With a position, the selection, cursor and anchor come back. The scroll offset comes
    /// back if it was measured in the current layout; otherwise the cursor scrolls into view.
    /// While the listing loads, the restore waits for [`CollectionState::sync`] with a
    /// complete listing; a press, a key or a scroll in the collection before then cancels it,
    /// and so does a new layout key.
    ///
    /// Either way, the controllers' gestures (an open marquee) and keyboard mode end, and the
    /// view moves only by instant scrolls (an offset, an instant reveal, or the top), each of
    /// which stops a glide in progress: a controller's `reset`, which has no [`Ctx`], never
    /// leaves an auto-scroll glide running.
    ///
    /// [`Ctx`]: super::Ctx
    ///
    /// With none, the selection clears and the view starts at the top. Picking an entry
    /// (the folder you came from, the first entry) is the host's policy.
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
        self.keyboard = false;
        self.hover_live = true;
        let changed = match position {
            Some(position) if listing == Listing::Loading => {
                self.pending = Some(position.clone());
                self.pending_offset = 0.0;
                let changed = self.selection.restore(&Position::default());
                tx.scroll_to_instant(viewport_path(scope), 0.0, 0.0);
                changed | self.selection.sync(source, SyncRule::loading())
            }
            Some(position) => self.apply_position(scope, source, position, SyncRule::default(), tx),
            None => {
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
                // Instant either way, and either way a scroll request, which stops a glide in
                // progress: the reveal when the row is out of view, else a scroll to where the
                // view is (a reveal that has nowhere to go might leave a glide running).
                Some(index) if self.shows_line(index, source.len()) => {
                    tx.scroll_to_instant(viewport_path(scope), 0.0, self.scroll.get());
                }
                Some(index) => self.reveal(scope, source, index, tx),
                None => tx.scroll_to_instant(viewport_path(scope), 0.0, 0.0),
            },
        }
        changed
    }
}
