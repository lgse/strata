// SPDX-License-Identifier: MIT

//! What the filter ([`crate::filter`]) asks of the Columns view: which column it opens in,
//! its results in the place of that column's rows, and the selection and the cursor over them.
//! GTK's column swaps its model for the search's hits and back
//! (`search_recursively` and `deactivate_search` in `src/ui/browser/columns/append/query.rs`).

use std::collections::HashMap;

use rune::Tx;
use strata_components::{
    collection::selection::{AnchorRule, Position, Selection},
    contract::{EntryId, EntrySource},
};
use strata_fs::app::Browser;
use strata_kit::look::Look;

use super::Columns;
use crate::{filter::HitsUpdate, listing::Column};

impl Columns {
    /// The column that holds the keyboard cursor in the strip.
    pub(crate) fn active(&self) -> usize {
        self.state.active()
    }

    /// Makes column `depth` the strip's active column when the filter opens in its header, as
    /// GTK's `focus.connect_enter` on the column shell does (`columns/append/surface.rs`).
    pub(crate) fn activate_for_filter(&mut self, depth: usize, look: &Look, tx: &mut Tx) {
        if depth < self.open() && depth != self.state.active() {
            self.state.set_active(depth, look, tx);
        }
    }

    /// The column a filter opens in: the one the pointer or the keys last targeted, GTK's
    /// `destination_depth`.
    pub(crate) fn targeted(&self) -> usize {
        self.state.targeted()
    }

    /// Keeps column `depth`'s header, where the filter's field is, the one that shows its
    /// actions while the pointer crosses the others; `None` lets the pointer target them again.
    pub(crate) fn hold_header(&mut self, depth: Option<usize>) {
        self.state.hold_target(depth);
    }

    /// Column `depth`'s folder rows, results or not: what the filter's placeholder counts.
    pub(crate) fn listing(&self, depth: usize) -> Option<&Column> {
        self.listings.columns.get(depth)
    }

    /// A folder the filter entered opens no child column for the cursor the browser places
    /// in it: GTK's column, its field holding the keyboard, mirrors nothing there.
    pub(crate) fn mirror_no_landing(&self) {
        self.mirror.note_input(false);
    }

    /// Whether column `depth`'s folder has been read.
    pub(crate) fn listed(&self, depth: usize) -> bool {
        self.listings
            .columns
            .get(depth)
            .is_some_and(|column| *column.load() == crate::listing::Load::Listed)
    }

    /// Whether a filter's results show in column `depth`.
    pub(crate) fn shows_hits(&self, depth: usize) -> bool {
        self.listings.hits_at(depth)
    }

    /// What column `depth` shows now, and the selection over it: what the filter's Enter and
    /// Down read.
    pub(crate) fn shown_with_selection(
        &self,
        depth: usize,
    ) -> Option<(&dyn EntrySource, &Selection)> {
        let shown = self.listings.shown(depth)?;
        let state = self.state.column(depth)?;
        Some((shown, &state.collection.selection))
    }

    /// Column `depth`'s selection over its folder's rows: what closing the filter gives back.
    pub(crate) fn listing_position(&self, depth: usize) -> Position {
        let (Some(column), Some(state)) =
            (self.listings.columns.get(depth), self.state.column(depth))
        else {
            return Position::default();
        };
        state.collection.selection.position(column)
    }

    /// Each result's path, by its id, and whether they show under the names, for the next
    /// results the filter shows.
    pub(crate) fn describe_hits(&mut self, paths: HashMap<EntryId, String>, details: bool) {
        self.listings.describe_hits(paths, details);
    }

    /// Shows `update.hits` in the place of column `depth`'s rows, with nothing selected when
    /// `update.clear` (a changed query over results that replace the listing) and a renamed
    /// result relocated before syncing.
    pub(crate) fn show_hits(&mut self, depth: usize, update: HitsUpdate, tx: &mut Tx) {
        if depth >= self.open() {
            return;
        }
        let revealed = self.relocate_hit(depth, &update);
        self.listings.show_hits(depth, update.hits);
        if update.clear
            && let Some(state) = self.state.column_mut(depth)
        {
            let _ = state.collection.selection.clear();
        }
        self.sync(depth, tx);
        self.note(depth);
        if let Some(index) = revealed.and_then(|id| self.listings.shown(depth)?.index_of(id)) {
            self.reveal_index(depth, index, tx);
        }
    }

    fn relocate_hit(&mut self, depth: usize, update: &HitsUpdate) -> Option<EntryId> {
        let old = self
            .listings
            .shown(depth)
            .filter(|_| self.listings.hits_at(depth))?;
        let (from, to) = update.relocation(old)?;
        let moved = self
            .state
            .column_mut(depth)?
            .collection
            .selection
            .relocate(from, to);
        (moved.selection || moved.cursor).then_some(to)
    }

    /// Gives the column the results showed in its rows back, with the selection `restore`
    /// it had before the filter, when there is one, and resynced to what `browser` holds now
    /// as GTK's `resync_column_selection` does (`src/ui/browser/columns/append/query.rs`;
    /// finding lgse/strata#2420).
    pub(crate) fn drop_hits(&mut self, restore: Option<&Position>, browser: &Browser, tx: &mut Tx) {
        let Some(depth) = self.listings.drop_hits() else {
            return;
        };
        self.sync(depth, tx);
        if let (Some(position), Some(state)) = (restore, self.state.column_mut(depth)) {
            let _ = state.collection.selection.restore(position);
        }
        self.note(depth);
        let marks = crate::events::marks(browser, depth, &self.ids);
        self.adopt(depth, &marks);
        self.caught_up();
    }

    /// The results column `depth` shows, taken out with their selection, for the view a switch
    /// shows. The column's rows show again.
    pub(crate) fn take_hits(&mut self, depth: usize) -> Option<(Column, Position)> {
        if !self.listings.hits_at(depth) {
            return None;
        }
        let position = self.listings.shown(depth).and_then(|hits| {
            let state = self.state.column(depth)?;
            Some(state.collection.selection.position(hits))
        });
        let (_, hits) = self.listings.take_hits()?;
        self.hold_header(None);
        let now = self.held_now(depth);
        if let Some(held) = self.held.get_mut(depth) {
            *held = now;
        }
        Some((hits, position.unwrap_or_default()))
    }

    /// The selection a view switch carried over, on the results column `depth` shows.
    pub(crate) fn restore_hits_selection(&mut self, depth: usize, position: &Position) {
        if let Some(state) = self.state.column_mut(depth) {
            let _ = state.collection.selection.restore(position);
        }
        self.note(depth);
    }

    /// Scrolls column `depth`'s row at `index` into view.
    pub(crate) fn reveal_index(&self, depth: usize, index: usize, tx: &mut Tx) {
        let (Some(shown), Some(state), Some(key)) = (
            self.listings.shown(depth),
            self.state.column(depth),
            self.state.column_key(depth),
        ) else {
            return;
        };
        state.collection.reveal(&key, shown, index, tx);
    }

    /// Down from the filter's field: the cursor on result `id`, which alone is selected, in
    /// the active column.
    pub(crate) fn land(&mut self, depth: usize, id: EntryId, look: &Look, tx: &mut Tx) {
        self.select_only((depth, id), look, tx);
    }

    /// Moves column `depth`'s cursor to `index` without replacing any committed selection.
    pub(crate) fn move_cursor_to(&mut self, depth: usize, index: usize, tx: &mut Tx) {
        let Some(shown) = self.listings.shown(depth) else {
            return;
        };
        let Some(key) = self.state.column_key(depth) else {
            return;
        };
        let Some(state) = self.state.column_mut(depth) else {
            return;
        };
        let _ = state
            .collection
            .selection
            .move_cursor(shown, index, AnchorRule::Keep);
        state.collection.note_selection(shown);
        state.collection.reveal(&key, shown, index, tx);
    }

    fn note(&mut self, depth: usize) {
        if let (Some(shown), Some(state)) =
            (self.listings.shown(depth), self.state.column_mut(depth))
        {
            state.collection.note_selection(shown);
        }
    }
}
