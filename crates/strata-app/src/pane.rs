// SPDX-License-Identifier: MIT

use std::rc::Rc;

use strata_components::{
    collection::{
        element::{CollectionState, Group, Groups},
        selection::{Listing, SyncRule},
    },
    contract::{Entry, EntryId, EntrySource, ThumbKey},
};
use strata_core::model::ViewPreferences;
use strata_fs::app::Browser;

use crate::{
    events::{Added, Change, Marks, Moved},
    handover::{self, Held, Kept, Moving, Shown},
    listing::{Column, Edit, Load},
    slots::Worn,
};

mod restore;

pub(crate) use restore::Memory;
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
    Returned,
}

pub(crate) struct Pane {
    /// The collection's key: its rows' keys and its scroll requests start there.
    scope: &'static str,
    /// A listing for each of the browser's columns, root first.
    columns: Vec<Column>,
    /// The window's saved item colours and icons, indexed by [`EntryId`].
    worn: Rc<Worn>,
    /// The browser's column shown: its active one, once it has one.
    shown: Option<usize>,
    /// Moves with every folder shown, so two folders of one length never share a version.
    generation: u64,
    collection: CollectionState,
    held: Held,
    /// The neighbour kept after a move out of the folder ([`handover::Kept`]).
    kept: Kept,
    moving: Moving,
    /// The collection has groups: the folder shown was grouped when they were last given.
    grouped: bool,
    /// Where each folder was left, for when it shows again.
    restore: Restore,
    restored: Option<Held>,
}

fn groups_of(column: &Column) -> Groups {
    let groups = column
        .group_runs()
        .iter()
        .map(|&(group, len)| Group::new(group.key(), group.translated_label(), len));
    Groups::new(groups)
}

impl Pane {
    pub(crate) fn share_memory(&mut self, memory: &Memory) {
        self.restore.share(memory);
    }

    pub(crate) fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        for column in &self.columns {
            column.collect_ids(out);
        }
        if let Some(id) = self.kept.id() {
            out.extend(std::iter::once(id));
        }
        self.moving.collect_ids(out);
    }

    pub(crate) fn new(scope: &'static str, collection: CollectionState) -> Self {
        Self {
            scope,
            columns: Vec::new(),
            worn: Worn::empty(),
            shown: None,
            generation: 0,
            held: Held::of(&collection),
            kept: Kept::default(),
            collection,
            moving: Moving::default(),
            grouped: false,
            restore: Restore::default(),
            restored: None,
        }
    }

    pub(crate) fn with_worn(mut self, worn: Rc<Worn>) -> Self {
        self.worn = worn;
        self
    }

    pub(crate) fn set_worn(&mut self, worn: &Rc<Worn>) {
        self.worn = worn.clone();
        for column in &mut self.columns {
            column.set_worn(worn);
        }
    }

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
            results: Vec::new(),
            drawn: Vec::new(),
            kept: self.shown.zip(self.kept.id()).into_iter().collect(),
        }
    }

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

    pub(crate) fn parts_mut(&mut self) -> (&dyn EntrySource, &mut CollectionState) {
        let source: &dyn EntrySource = match self.shown.and_then(|at| self.columns.get(at)) {
            Some(column) => column,
            None => &Nothing,
        };
        (source, &mut self.collection)
    }

    /// Moves only when the shown row keys or their order change (`virtual_rows_version`).
    pub(crate) fn key_version(&self) -> u64 {
        let version = self.listing().map_or(0, Column::key_version);
        self.generation.wrapping_shl(40) ^ version
    }

    /// Moves when row keys, order, thumbnail keys, or worn customizations change.
    pub(crate) fn thumb_version(&self) -> u64 {
        let version = self.listing().map_or(0, Column::thumb_version);
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
                self.moving.moved_in(from, to, &self.columns);
            }
            within => self.apply_within(within, tx),
        }
        match before == (self.shown, self.generation) {
            true => Applied::Within,
            false if self.restore.returned() => Applied::Returned,
            false => Applied::Entered,
        }
    }

    fn follow_grouping(&mut self, depth: usize, grouping: Option<ViewPreferences>) {
        if let Some(column) = self.columns.get_mut(depth) {
            column.set_grouping(grouping);
        }
    }

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
        let mut column = Column::new(location, show_hidden, None).with_worn(self.worn.clone());
        column.set_grouping(grouping);
        self.columns.push(column);
        if active || self.shown == Some(depth) {
            self.show(Some(depth), reveal, tx);
        }
    }

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
        let before = self.columns.get(depth).map(Column::key_version);
        self.follow_grouping(depth, grouping);
        let Some(column) = self.columns.get_mut(depth) else {
            return;
        };
        column.apply(edit);
        if self.shown != Some(depth) {
            return;
        }
        if marks.is_none() && before == Some(column.key_version()) {
            return;
        }
        self.arrived();
        match marks {
            Some(marks) => self.settle(&marks, tx),
            None => self.sync(tx),
        }
    }

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

    pub(crate) fn scroll_to_top(&self, tx: &mut rune::Tx) {
        tx.scroll_to_instant(
            strata_components::collection::element::viewport_path(self.scope),
            0.0,
            0.0,
        );
    }

    /// Brings the collection in step with its rows after they changed.
    fn sync(&mut self, tx: &mut rune::Tx) {
        self.sync_by(handover::sync_rule, tx);
    }

    /// [`Pane::sync`] by `rule`: [`handover::settle_rule`] when the browser's marks follow.
    fn sync_by(&mut self, rule: fn(&Column) -> SyncRule, tx: &mut rune::Tx) {
        self.regroup();
        let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) else {
            return;
        };
        let _ = self.collection.sync(self.scope, column, rule(column), tx);
    }

    fn settle_at(&mut self, depth: usize, marks: &Marks, tx: &mut rune::Tx) {
        if self.shown == Some(depth) {
            self.settle(marks, tx);
        }
    }

    fn settle(&mut self, marks: &Marks, tx: &mut rune::Tx) {
        let before = self.collection.selection.cursor();
        let restoring = self.collection.restore_pending();
        let rule = match restoring {
            true => handover::sync_rule,
            false => handover::settle_rule,
        };
        self.sync_by(rule, tx);
        let restored = restoring && !self.collection.restore_pending();
        if restored {
            if let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) {
                restore::fall_back(column, &mut self.collection, marks.cursor);
                restore::take_load_cursor(column, &mut self.collection, marks);
                restore::keep_provisional(column, &mut self.collection, marks);
            }
            self.restored = Some(Held::of(&self.collection));
        } else if !handover::placed(before, &self.collection) {
            let kept = self
                .shown
                .and_then(|depth| self.columns.get(depth))
                .and_then(|column| {
                    handover::kept_neighbour(before, column, &self.collection, marks)
                });
            self.adopt(kept.as_ref().unwrap_or(marks));
            if kept.is_some() {
                self.kept.keep(kept.as_ref());
            }
        }
    }

    fn adopt(&mut self, marks: &Marks) {
        let Some(column) = self.shown.and_then(|depth| self.columns.get(depth)) else {
            return;
        };
        let kept = self.kept.marks(&self.collection, marks);
        let marks = kept.as_ref().unwrap_or(marks);
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
        let restored = self.restored.take() == Some(Held::of(&self.collection));
        match restored {
            true => handover::hand_back(&shown, browser, &mut self.held),
            false => handover::hand_over(&shown, browser, &mut self.held),
        }
        let followed = restored
            && (restore::follow_fill(column, &mut self.collection, browser, depth)
                | restore::anchor_at_cursor(column, &mut self.collection));
        if followed {
            self.held = Held::of(&self.collection);
        }
    }

    /// Thumbnail `(from, to)` keys to relocate in the window's store after an arrived rename.
    pub(crate) fn take_relocated_thumbnails(&mut self) -> Vec<(ThumbKey, ThumbKey)> {
        self.moving.take_thumbnails()
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
