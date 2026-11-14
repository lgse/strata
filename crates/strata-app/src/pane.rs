// SPDX-License-Identifier: MIT

//! The one folder a single-pane view shows, for the Icons and List views: the browser's
//! active column. GTK's Icons and List views rebuild their pane from `active_depth` whenever
//! the columns change (`rebuild_icons` and `rebuild_list` in `src/ui/browser_modes/rebuild.rs`,
//! called from `ColumnsTruncated`, `ColumnsRelocated` and an active `ColumnAdded` in
//! `src/ui/browser_modes/events.rs`), and this does the same. Shared code, owned by the
//! contract ([`crate::slots`], "Who owns what"). It knows nothing of a grid or a table: a view
//! keeps its component's own state beside it.
//!
//! - **Events in.** A view's listener queues the changes [`crate::events`] reads, and
//!   [`Pane::apply`] puts them into its listings, one for each of the browser's columns, so
//!   that whichever the browser makes active can show at once. The collection shows the active
//!   one, chosen on the same events as GTK's: a reset, an active column added, and columns
//!   closing. A cursor moving to another column (`Browser::set_active_column` raises nothing
//!   of its own) leaves the shown one, as it leaves GTK's pane.
//! - **One cursor and one selection,** kept through [`crate::handover`], as each column of the
//!   Columns view keeps them: the collection interprets keys and the pointer, the browser
//!   holds the result, and [`Pane::publish`] hands it over. The collection is reached only
//!   through the pane, so nothing changes it outside that bookkeeping but the component's own
//!   handlers ([`Pane::parts_mut`]).
//! - **Order.** A view index is the listing's ([`Column`]): the browser's order, or, where the
//!   browser groups the folder by file type (`Browser::shows_type_groups`, which only a view
//!   that supports type grouping turns on), strata-fs's group order, each group keeping the
//!   browser's order. The collection, its cursor, anchor, ranges and keys, and so
//!   [`Pane::neighbour`], work in view indices; what crosses to the browser goes by id
//!   ([`Pane::position`], the handover). The pane gives the collection the groups before
//!   each `enter` and `sync`, and none when the folder is not grouped.
//! - **One pane for the window.** The collection is keyed by a fixed `scope`; a window with
//!   tabs will need one pane, and one key, for each tab.

use std::sync::Arc;

use strata_components::{
    collection::{
        element::{CollectionState, Group, Groups},
        selection::Listing,
    },
    contract::{Entry, EntryId, EntrySource},
};
use strata_core::{i18n::tr, model::ViewPreferences};
use strata_fs::app::Browser;

use crate::{
    events::{Added, Change, Marks, Moved},
    handover::{self, Held, Moving, Shown},
    listing::{Column, Edit, Load},
};

mod restore;

use restore::Restore;

/// What a pane shows before the browser has opened a folder: nothing.
struct Nothing;

impl EntrySource for Nothing {
    fn len(&self) -> usize {
        0
    }

    fn id_at(&self, index: usize) -> EntryId {
        unreachable!("an empty listing has no entry {index}")
    }

    fn index_of(&self, _: EntryId) -> Option<usize> {
        None
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        unreachable!("an empty listing has no entry {index}")
    }
}

/// What applying a change did to the folder shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The same folder shows: its rows or its marks may have changed.
    Within,
    /// Another folder shows, or none does.
    Entered,
    /// Another folder shows, at the position remembered for it when it was left
    /// ([`restore`]): a first visit does not arm for it, as GTK's waits for no saved visit.
    Returned,
}

pub(crate) struct Pane {
    /// The collection's key: its rows' keys and its scroll requests start there.
    scope: &'static str,
    /// A listing for each of the browser's columns, root first.
    columns: Vec<Column>,
    /// The browser's column shown: its active one, once it has one.
    shown: Option<usize>,
    /// Moves with every folder shown, so two folders of one length never share a version.
    generation: u64,
    collection: CollectionState,
    held: Held,
    moving: Moving,
    /// The collection has groups: the folder shown was grouped when they were last given.
    grouped: bool,
    /// Where each folder was left, for when it shows again.
    restore: Restore,
}

/// The groups of `column`'s rows as it shows them: one for each run of a file type, headed by
/// its description, translated once for the group.
fn groups_of(column: &Column) -> Groups {
    let mut groups: Vec<Group> = Vec::new();
    let mut current = None;
    for index in 0..column.len() {
        let Some(group) = column.group_at(index) else {
            continue;
        };
        match (current, groups.last_mut()) {
            (Some(open), Some(last)) if open == group => last.len += 1,
            _ => {
                let label: Arc<str> = Arc::from(tr(&group.description()));
                groups.push(Group::new(group.key(), label, 1));
                current = Some(group);
            }
        }
    }
    Groups::new(groups)
}

impl Pane {
    pub(crate) fn new(scope: &'static str, collection: CollectionState) -> Self {
        Self {
            scope,
            columns: Vec::new(),
            shown: None,
            generation: 0,
            held: Held::of(&collection),
            collection,
            moving: Moving::default(),
            grouped: false,
            restore: Restore::default(),
        }
    }

    /// The view's layout, as C1e keys a scroll offset: an offset comes back only in the
    /// layout it was measured in. The view sets it before each change applies.
    pub(crate) fn set_layout_key(&mut self, layout: u64) {
        self.collection.set_layout_key(layout);
    }

    /// The browser's column shown.
    pub(crate) fn depth(&self) -> Option<usize> {
        self.shown
    }

    pub(crate) fn listing(&self) -> Option<&Column> {
        self.columns.get(self.shown?)
    }

    /// What the pane holds of the browser's columns: every listing, and the one shown with
    /// its collection.
    #[cfg(feature = "qa")]
    pub(crate) fn presented(&self) -> handover::Presented<'_> {
        let shown = self.shown.and_then(|depth| {
            Some(Shown {
                depth,
                column: self.columns.get(depth)?,
                collection: &self.collection,
            })
        });
        let pending = self
            .shown
            .filter(|_| self.held != Held::of(&self.collection))
            .into_iter()
            .collect();
        handover::Presented {
            active: self.shown,
            strip: None,
            listings: self.columns.iter().collect(),
            shown: shown.into_iter().collect(),
            pending,
        }
    }

    /// Whether the folder shown has been read: a first visit waits on it, and the QA probe's
    /// `ready` reads it.
    pub(crate) fn load(&self) -> Option<&Load> {
        self.listing().map(Column::load)
    }

    pub(crate) fn collection(&self) -> &CollectionState {
        &self.collection
    }

    /// The listing as the collection reads it: an empty one while no folder shows.
    pub(crate) fn source(&self) -> &dyn EntrySource {
        match self.listing() {
            Some(column) => column,
            None => &Nothing,
        }
    }

    /// The listing and the collection, as the component's handlers take them: what they
    /// change in the selection reaches the browser at the next [`Pane::publish`].
    pub(crate) fn parts_mut(&mut self) -> (&dyn EntrySource, &mut CollectionState) {
        let source: &dyn EntrySource = match self.shown.and_then(|at| self.columns.get(at)) {
            Some(column) => column,
            None => &Nothing,
        };
        (source, &mut self.collection)
    }

    /// The collection's listing version: it moves with every change to what shows.
    pub(crate) fn version(&self) -> u64 {
        let version = self.listing().map_or(0, Column::version);
        self.generation.wrapping_shl(40) ^ version
    }

    /// The listing of column `depth`, when it is the one shown.
    fn shown_column(&self, depth: usize) -> Option<&Column> {
        self.listing().filter(|_| self.shown == Some(depth))
    }

    /// The browser's position of entry `id` in column `depth`, when that column shows.
    pub(crate) fn position(&self, depth: usize, id: EntryId) -> Option<usize> {
        self.shown_column(depth)?.position_of(id)
    }

    /// The entry `step` entries after `id` in column `depth`, in the listing's order.
    pub(crate) fn neighbour(&self, depth: usize, id: EntryId, step: isize) -> Option<EntryId> {
        let column = self.shown_column(depth)?;
        let index = column.index_of(id)?.checked_add_signed(step)?;
        (index < column.len()).then(|| column.id_at(index))
    }

    /// Applies one change the view's listener queued. A change to a column's rows or load
    /// carries how the browser groups its folder, read when the event came, which the column
    /// follows before the change applies.
    pub(crate) fn apply(&mut self, change: Change, tx: &mut rune::Tx) -> Applied {
        let before = (self.shown, self.generation);
        if matches!(
            change,
            Change::Reset | Change::Truncated { .. } | Change::Added(_)
        ) {
            let shown = self.shown.and_then(|depth| self.columns.get(depth));
            self.restore.leave(shown, &self.collection);
        }
        match change {
            Change::Reset => {
                self.columns.clear();
                self.show(None, false, tx);
            }
            Change::Truncated { len, active, .. } => self.truncate(len, active, tx),
            Change::Added(added) => self.add(added, tx),
            Change::Relocated {
                from_depth,
                columns,
            } => self.relocate(from_depth, columns, tx),
            Change::Moved { from, to } => {
                let columns = &self.columns;
                let lists = |id: EntryId| columns.iter().any(|c| c.position_of(id).is_some());
                self.moving.moved(from, to, lists);
            }
            within => self.apply_within(within, tx),
        }
        match before == (self.shown, self.generation) {
            true => Applied::Within,
            false if self.restore.returned() => Applied::Returned,
            false => Applied::Entered,
        }
    }

    /// Column `depth` follows how the browser groups its folder, before a change to it
    /// applies.
    fn follow_grouping(&mut self, depth: usize, grouping: Option<ViewPreferences>) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.set_grouping(grouping);
        }
    }

    /// Gives the collection the groups of the folder shown, or takes them away: before each
    /// `enter` and `sync`, as the collection asks.
    fn regroup(&mut self) {
        let column = self.shown.and_then(|depth| self.columns.get(depth));
        match column.filter(|column| column.grouping().is_some()) {
            Some(column) => {
                self.collection.set_groups(groups_of(column));
                self.grouped = true;
            }
            None if self.grouped => {
                self.collection.set_groups(Groups::default());
                self.grouped = false;
            }
            None => {}
        }
    }

    fn apply_within(&mut self, change: Change, tx: &mut rune::Tx) {
        match change {
            rows @ Change::Rows { .. } => self.edit(rows, tx),
            Change::Loaded {
                depth,
                load,
                marks,
                grouping,
            } => {
                self.follow_grouping(depth, grouping);
                if let Some(column) = self.columns.get_mut(depth) {
                    column.set_load(load);
                    column.set_busy(false);
                }
                self.settle_at(depth, &marks, tx);
            }
            Change::Busy { depth, busy } => {
                if let Some(column) = self.columns.get_mut(depth) {
                    column.set_busy(busy);
                }
            }
            Change::Hidden { show_hidden, marks } => {
                for column in &mut self.columns {
                    column.set_show_hidden(show_hidden);
                }
                if let Some(marks) = self.shown.and_then(|depth| marks.get(depth)) {
                    self.settle(marks, tx);
                }
            }
            selection @ Change::Selection { .. } => self.selected(selection, tx),
            // `apply` took the rest.
            _ => {}
        }
    }

    /// Shows column `depth`, or nothing, at the position remembered for it unless the
    /// browser `reveal`s an entry there.
    fn show(&mut self, depth: Option<usize>, reveal: bool, tx: &mut rune::Tx) {
        let depth = depth.filter(|depth| *depth < self.columns.len());
        self.shown = depth;
        self.generation += 1;
        self.regroup();
        let column = depth.and_then(|depth| self.columns.get(depth));
        let position = self.restore.arrive(column, reveal);
        let (source, listing) = match column {
            Some(column) if column.is_loading() => (column as &dyn EntrySource, Listing::Loading),
            Some(column) => (column as &dyn EntrySource, Listing::Complete),
            None => (&Nothing as &dyn EntrySource, Listing::Complete),
        };
        let _ = self
            .collection
            .enter(self.scope, source, position.as_ref(), listing, tx);
        self.held = Held::of(&self.collection);
    }

    /// The columns from `len` closed. GTK rebuilds its pane from the browser's active column
    /// then, so the active column shows if it is another. An active column not added yet (a
    /// sibling's `Added` follows) leaves the shown one, as GTK's rebuild finds no snapshot for
    /// it and keeps its pane, unless the shown one closed.
    fn truncate(&mut self, len: usize, active: Option<usize>, tx: &mut rune::Tx) {
        self.columns.truncate(len);
        match active.filter(|active| *active < len) {
            Some(active) if self.shown != Some(active) => self.show(Some(active), false, tx),
            Some(_) => {}
            None if self.shown.is_some_and(|shown| shown >= len) => self.show(None, false, tx),
            None => {}
        }
    }

    fn add(&mut self, added: Added, tx: &mut rune::Tx) {
        let Added {
            depth,
            location,
            show_hidden,
            active,
            grouping,
            reveal,
        } = added;
        self.columns.truncate(depth);
        // The browser adds columns in order, each after the last.
        if self.columns.len() != depth {
            return;
        }
        let mut column = Column::new(location, show_hidden, None);
        column.set_grouping(grouping);
        self.columns.push(column);
        // Opening a folder in place makes the new root active, and history the column it
        // restores. A column opened with its parent kept active (`descend`) does not show.
        // The browser closes a column before it adds another at its depth, so `truncate` has
        // already left a shown column of this depth; should it not have, the new folder
        // shows in its place rather than the old one's rows under the new listing.
        if active || self.shown == Some(depth) {
            self.show(Some(depth), reveal, tx);
        }
    }

    /// Folders that moved keep their listings, under their new names and with every row read
    /// again, since a row's id comes from its location.
    fn relocate(&mut self, from_depth: usize, moved: Vec<Moved>, tx: &mut rune::Tx) {
        for (depth, moved) in (from_depth..).zip(moved) {
            let Some(column) = self.columns.get_mut(depth) else {
                return;
            };
            column.relocate(moved.location, None);
            column.set_grouping(moved.grouping);
            column.apply(Edit::Replace(moved.rows));
            self.settle_at(depth, &moved.marks, tx);
        }
    }

    fn edit(&mut self, rows: Change, tx: &mut rune::Tx) {
        let Change::Rows {
            depth,
            edit,
            marks,
            grouping,
        } = rows
        else {
            return;
        };
        self.follow_grouping(depth, grouping);
        let Some(column) = self.columns.get_mut(depth) else {
            return;
        };
        column.apply(edit);
        if self.shown != Some(depth) {
            return;
        }
        self.arrived();
        match marks {
            Some(marks) => self.settle(&marks, tx),
            None => self.sync(tx),
        }
    }

    /// The browser's cursor or selection changed in column `depth`. It never changes which
    /// column shows: GTK's `on_focus_changed` only moves the selection and the focus.
    fn selected(&mut self, selection: Change, tx: &mut rune::Tx) {
        let Change::Selection {
            depth,
            marks,
            reveal,
            ..
        } = selection
        else {
            return;
        };
        if self.shown != Some(depth) {
            return;
        }
        self.adopt(&marks);
        if reveal {
            self.reveal_cursor(tx);
        }
    }

    /// Scrolls the cursor's entry into view, as the browser asks of a change it makes.
    fn reveal_cursor(&self, tx: &mut rune::Tx) {
        let Some(column) = self.listing() else {
            return;
        };
        if let Some(index) = self.collection.selection.cursor_index(column) {
            self.collection.reveal(self.scope, column, index, tx);
        }
    }

    /// Brings the collection in step with its rows after they changed.
    fn sync(&mut self, tx: &mut rune::Tx) {
        self.regroup();
        let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) else {
            return;
        };
        let rule = handover::sync_rule(column);
        let _ = self.collection.sync(self.scope, column, rule, tx);
    }

    fn settle_at(&mut self, depth: usize, marks: &Marks, tx: &mut rune::Tx) {
        if self.shown == Some(depth) {
            self.settle(marks, tx);
        }
    }

    /// After the rows changed: the collection catches up with them, and then the browser's
    /// cursor and selection stand, but for a cursor the collection placed itself
    /// ([`handover::placed`]) and a remembered position that came back ([`restore`]).
    fn settle(&mut self, marks: &Marks, tx: &mut rune::Tx) {
        let before = self.collection.selection.cursor();
        let restoring = self.collection.restore_pending();
        self.sync(tx);
        // A remembered position that came back with the listing is the pane's, as GTK's
        // restore sets the browser's selection to it: the browser's marks don't replace it.
        let restored = restoring && !self.collection.restore_pending();
        if restored {
            if let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) {
                restore::fall_back(column, &mut self.collection, marks.cursor);
            }
        } else if !handover::placed(before, &self.collection) {
            self.adopt(marks);
        }
    }

    fn adopt(&mut self, marks: &Marks) {
        let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) else {
            return;
        };
        self.held = handover::adopt(column, &mut self.collection, marks);
    }

    /// The shown rows changed: what the selection holds by an old id it holds by the new one.
    fn arrived(&mut self) {
        let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) else {
            return;
        };
        let _ = self.moving.arrived(column, &mut self.collection);
        let columns = &self.columns;
        self.moving
            .forget(|id| columns.iter().any(|c| c.position_of(id).is_some()));
    }

    /// Hands the browser what the view changed since it last agreed with it.
    pub(crate) fn publish(&mut self, browser: &Browser) {
        let Some((depth, column)) = self
            .shown
            .and_then(|depth| Some((depth, self.columns.get(depth)?)))
        else {
            return;
        };
        let shown = Shown {
            depth,
            column,
            collection: &self.collection,
        };
        handover::hand_over(&shown, browser, &mut self.held);
    }

    /// Selects entry `id` of column `depth` alone, with the cursor on it.
    pub(crate) fn select_only(&mut self, depth: usize, id: EntryId) {
        let Some(column) = self
            .columns
            .get(depth)
            .filter(|_| self.shown == Some(depth))
        else {
            return;
        };
        let Some(index) = column.index_of(id) else {
            return;
        };
        let _ = self.collection.selection.select_only(column, index);
        self.collection.note_selection(column);
    }
}

#[cfg(test)]
mod tests;
