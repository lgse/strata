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
use strata_core::model::Location;
use strata_fs::app::Browser;
use strata_kit::look::Look;

use super::Columns;

impl Columns {
    /// The id the view knows the entry at `location` by.
    pub(crate) fn id_of(&self, location: &Location) -> EntryId {
        self.ids.of(location)
    }

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
    /// entry another location. The listing shows it in a later event, and a selection may
    /// only be told once its listing has changed, so the pair waits for [`Columns::arrived`].
    /// A listing that changed first, as when the folder's monitor reported the rename before
    /// the operation did, has let go of the old id already.
    pub(super) fn moved(&mut self, from: EntryId, to: EntryId) {
        let lists = |id: EntryId| {
            let mut columns = self.listings.columns.iter();
            columns.any(|column| column.position_of(id).is_some())
        };
        if lists(from) && !lists(to) {
            self.moving.push((from, to));
        }
    }

    /// Column `depth`'s rows changed, and its selection has not been brought in step with
    /// them yet. What it holds by the old id of an entry that now shows under its new one it
    /// holds by the new one: a range keeps its anchor and its members through a rename.
    pub(super) fn arrived(&mut self, depth: usize) {
        let Some(column) = self.listings.columns.get(depth) else {
            return;
        };
        let here = |(from, to): &(EntryId, EntryId)| {
            column.position_of(*to).is_some() && column.position_of(*from).is_none()
        };
        let arrived: Vec<(EntryId, EntryId)> = self.moving.iter().copied().filter(here).collect();
        for (from, to) in arrived {
            self.rename_held(depth, from, to);
        }
        // A pair whose old id no column lists any more has nothing left to move.
        let columns = &self.listings.columns;
        let listed = |id: EntryId| {
            columns
                .iter()
                .any(|column| column.position_of(id).is_some())
        };
        self.moving.retain(|(from, _)| listed(*from));
    }

    /// Column `depth`'s selection, and the cursor the child column follows, hold `to` where
    /// they held `from`.
    fn rename_held(&mut self, depth: usize, from: EntryId, to: EntryId) {
        if let Some(state) = self.state.column_mut(depth) {
            let _ = state.collection.selection.relocate(from, to);
        }
        let followed = self.followed.as_mut().map(|(_, followed)| followed);
        if let Some(followed) = followed.filter(|followed| **followed == from) {
            *followed = to;
        }
    }
}
