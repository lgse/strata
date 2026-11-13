// SPDX-License-Identifier: MIT

//! Where the rename editor sits in the columns: the row it opens on, the listing it is
//! judged against, and the neighbour Tab carries it to. What a rename does is
//! [`crate::rename`]'s.

use std::rc::Rc;

use rune::Tx;
use strata_components::{
    contract::{EntryId, EntrySource},
    rename::{By, Initial, Polled, RenameState},
};
use strata_fs::app::Browser;
use strata_kit::look::Look;

use super::Columns;

impl Columns {
    /// The column whose collection is keyed `scope`, while it is open.
    fn depth_of(&self, scope: &str) -> Option<usize> {
        (0..self.open()).find(|depth| self.state.column_key(*depth).as_deref() == Some(scope))
    }

    /// The key of column `depth`'s collection, which names the edit's column to the editor.
    pub(crate) fn scope(&self, depth: usize) -> Option<String> {
        (depth < self.open())
            .then(|| self.state.column_key(depth))
            .flatten()
    }

    /// Opens `editor` on the row of entry `id` in column `depth` and shows the row. `None`
    /// when the column does not show the entry: a hidden one has no row to edit in.
    pub(crate) fn begin_rename(
        &self,
        editor: &mut RenameState,
        (depth, id): (usize, EntryId),
        tx: &mut Tx,
    ) -> Option<Polled> {
        let scope = self.scope(depth)?;
        let column = self.listings.columns.get(depth)?;
        let index = column.index_of(id)?;
        let entry = column.entry(index);
        let polled = editor.begin(&scope, id, entry.name, entry.kind, Initial::Stem);
        self.state
            .column(depth)?
            .collection
            .reveal(&scope, column, index, tx);
        Some(polled)
    }

    /// Scrolls the row of entry `id` in column `depth` into view, just far enough.
    pub(crate) fn reveal_row(&self, (depth, id): (usize, EntryId), tx: &mut Tx) {
        let (Some(scope), Some(column)) = (self.scope(depth), self.listings.columns.get(depth))
        else {
            return;
        };
        if let (Some(index), Some(state)) = (column.index_of(id), self.state.column(depth)) {
            state.collection.reveal(&scope, column, index, tx);
        }
    }

    /// One poll of `editor`, against the listing and the viewport of the column its edit is
    /// in. An edit whose column has closed ends, and keeps the name.
    pub(crate) fn tick_rename(&self, editor: &mut RenameState) -> Polled {
        let Some(scope) = editor.scope().map(str::to_owned) else {
            return Polled::default();
        };
        let parts = self.depth_of(&scope).and_then(|depth| {
            let column = self.listings.columns.get(depth)?;
            Some((column, &self.state.column(depth)?.collection))
        });
        match parts {
            Some((column, collection)) => editor.tick(&scope, column, collection),
            None => editor.cancel(By::Listing),
        }
    }

    /// The entry `step` rows after `id` in column `depth`, as the view lists them.
    pub(crate) fn neighbour(&self, depth: usize, id: EntryId, step: isize) -> Option<EntryId> {
        let column = self.listings.columns.get(depth)?;
        let index = column.index_of(id)?.checked_add_signed(step)?;
        (index < column.len()).then(|| column.id_at(index))
    }

    /// Selects entry `id` of column `depth` alone, with the cursor and the keyboard on it:
    /// what `Browser::select` does for the GTK views.
    pub(crate) fn select_only(&mut self, (depth, id): (usize, EntryId), look: &Look, tx: &mut Tx) {
        let (Some(column), Some(state)) = (
            self.listings.columns.get(depth),
            self.state.column_mut(depth),
        ) else {
            return;
        };
        let Some(index) = column.index_of(id) else {
            return;
        };
        let _ = state.collection.selection.select_only(column, index);
        state.collection.note_selection(column);
        if depth != self.state.active() {
            self.state.set_active(depth, look, tx);
        }
    }

    /// The child column follows the cursor at once, with no wait: Tab carried a rename to
    /// the cursor's entry, and the editor that opens on it holds the mirror back.
    pub(crate) fn mirror_now(&self, browser: &Rc<Browser>) {
        if let Some((depth, id)) = self.focused()
            && let Some(position) = self.position(depth, id)
        {
            self.mirror.now(browser, depth, position);
        }
    }

    /// An edit is open, or none is: the mirror waits while one is.
    pub(crate) fn hold_mirror(&self, editing: bool) {
        self.mirror.hold(editing);
    }

    /// One of the browser's operations gave the entry `from` the id `to`: a rename gives an
    /// entry another location ([`Moving`](crate::handover::Moving)).
    pub(super) fn moved(&mut self, from: EntryId, to: EntryId) {
        let columns = &self.listings.columns;
        let lists = |id: EntryId| {
            columns
                .iter()
                .any(|column| column.position_of(id).is_some())
        };
        self.moving.moved(from, to, lists);
    }

    /// Column `depth`'s rows changed, and its selection has not been brought in step with
    /// them yet: what it holds by an old id it holds by the new one, and so does the cursor
    /// the child column follows.
    pub(super) fn arrived(&mut self, depth: usize) {
        let Some(column) = self.listings.columns.get(depth) else {
            return;
        };
        let arrived = match self.state.column_mut(depth) {
            Some(state) => self.moving.arrived(column, &mut state.collection),
            None => self.moving.arriving(column),
        };
        for (from, to) in arrived {
            self.follow_moved(from, to);
        }
        // A pair whose old id no column lists any more has nothing left to move.
        let columns = &self.listings.columns;
        self.moving.forget(|id| {
            columns
                .iter()
                .any(|column| column.position_of(id).is_some())
        });
    }

    /// The cursor the child column follows holds `to` where it held `from`.
    fn follow_moved(&mut self, from: EntryId, to: EntryId) {
        let followed = self.followed.as_mut().map(|(_, followed)| followed);
        if let Some(followed) = followed.filter(|followed| **followed == from) {
            *followed = to;
        }
    }
}
