// SPDX-License-Identifier: MIT

//! What the filter ([`crate::filter`]) asks of the Columns view: which column it opens in,
//! its results in the place of that column's rows, and the selection and the cursor over them.
//! GTK's column swaps its model for the search's hits and back
//! (`search_recursively` and `deactivate_search` in `src/ui/browser/columns/append/query.rs`).

use rune::Tx;
use strata_components::{
    collection::selection::{Position, Selection},
    contract::{EntryId, EntrySource},
};
use strata_kit::look::Look;

use super::Columns;
use crate::listing::Column;

impl Columns {
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

    /// How many of its folder's rows column `depth` shows: what the filter's placeholder
    /// counts.
    pub(crate) fn listing_len(&self, depth: usize) -> usize {
        self.listings.columns.get(depth).map_or(0, EntrySource::len)
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
        let (Some(column), Some(state)) = (
            self.listings.columns.get(depth),
            self.state.column(depth),
        ) else {
            return Position::default();
        };
        state.collection.selection.position(column)
    }

    /// Shows `hits` in the place of column `depth`'s rows, with nothing selected when `clear`
    /// (a changed query over results that replace the listing).
    pub(crate) fn show_hits(&mut self, depth: usize, hits: Column, clear: bool, tx: &mut Tx) {
        if depth >= self.open() {
            return;
        }
        self.listings.show_hits(depth, hits);
        if clear && let Some(state) = self.state.column_mut(depth) {
            let _ = state.collection.selection.clear();
        }
        self.sync(depth, tx);
        self.note(depth);
    }

    /// Gives the column the results showed in its rows back, with the selection `restore`
    /// it had before the filter, when there is one.
    pub(crate) fn drop_hits(&mut self, restore: Option<&Position>, tx: &mut Tx) {
        let Some(depth) = self.listings.drop_hits() else {
            return;
        };
        self.sync(depth, tx);
        if let (Some(position), Some(state)) = (restore, self.state.column_mut(depth)) {
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

    fn note(&mut self, depth: usize) {
        if let (Some(shown), Some(state)) =
            (self.listings.shown(depth), self.state.column_mut(depth))
        {
            state.collection.note_selection(shown);
        }
    }
}
