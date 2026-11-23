// SPDX-License-Identifier: MIT

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

    pub(crate) fn activate_for_filter(&mut self, depth: usize, look: &Look, tx: &mut Tx) {
        if depth < self.open() && depth != self.state.active() {
            self.state.set_active(depth, look, tx);
        }
    }

    pub(crate) fn targeted(&self) -> usize {
        self.state.targeted()
    }

    pub(crate) fn hold_header(&mut self, depth: Option<usize>) {
        self.state.hold_target(depth);
    }

    /// Column `depth`'s folder rows, results or not: what the filter's placeholder counts.
    pub(crate) fn listing(&self, depth: usize) -> Option<&Column> {
        self.listings.columns.get(depth)
    }

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

    pub(crate) fn describe_hits(&mut self, paths: HashMap<EntryId, String>, details: bool) {
        self.listings.describe_hits(paths, details);
    }

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
