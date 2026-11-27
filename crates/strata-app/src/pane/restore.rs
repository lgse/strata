// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use strata_components::{
    collection::{
        element::CollectionState,
        selection::{AnchorRule, Position, RestoreMemory},
    },
    contract::{EntryId, EntrySource},
};
use strata_core::model::Location;
use strata_fs::app::Browser;

use crate::{
    events::Marks,
    listing::{Column, Load},
};

#[derive(Clone, Default)]
pub(crate) struct Memory(Rc<RefCell<Kept>>);

#[derive(Default)]
struct Kept {
    places: RestoreMemory<Location>,
    /// A view switched to is being rebuilt from the browser: the folder it leaves is stale.
    replaying: bool,
    visit: Option<(usize, EntryId)>,
    visit_rows: (usize, Option<EntryId>),
}

impl Memory {
    /// While `replaying`, a view leaving a folder does not remember it.
    pub(crate) fn set_replaying(&self, replaying: bool) {
        self.0.borrow_mut().replaying = replaying;
    }

    pub(crate) fn visit_cursor(&self) -> Option<(usize, EntryId)> {
        self.0.borrow().visit
    }

    /// The cursor a first visit placed is at `at` now, or there is none.
    pub(crate) fn set_visit_cursor(&self, at: Option<(usize, EntryId)>) {
        self.0.borrow_mut().visit = at;
    }

    pub(crate) fn visit_rows(&self) -> (usize, Option<EntryId>) {
        self.0.borrow().visit_rows
    }

    /// The folder shows `rows`, how many and the first, as the visit's cursor is placed or kept.
    pub(crate) fn set_visit_rows(&self, rows: (usize, Option<EntryId>)) {
        self.0.borrow_mut().visit_rows = rows;
    }

    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        let kept = self.0.borrow();
        kept.places.collect_ids(out);
        if let Some((_, id)) = kept.visit {
            out.extend(std::iter::once(id));
        }
        if let Some(id) = kept.visit_rows.1 {
            out.extend(std::iter::once(id));
        }
    }
}

#[derive(Default)]
pub(super) struct Restore {
    memory: Memory,
    /// The folder being left by the change applying now, and where it was.
    leaving: Option<(Location, Position)>,
    /// The folder shown last came back at a position remembered for it.
    returned: bool,
}

impl Restore {
    pub(super) fn leave(&mut self, shown: Option<&Column>, collection: &CollectionState) {
        self.returned = false;
        if self.memory.0.borrow().replaying {
            self.leaving = None;
            return;
        }
        self.leaving = shown
            .filter(|column| listed(column))
            .map(|column| (column.location().clone(), collection.position(column)));
    }

    pub(super) fn arrive(&mut self, column: Option<&Column>, reveal: bool) -> Option<Position> {
        let mut kept = self.memory.0.borrow_mut();
        if let Some((location, position)) = self.leaving.take() {
            let _ = kept.places.remember(location, position);
        }
        let position = column
            .filter(|column| !reveal && column.is_loading())
            .and_then(|column| kept.places.recall(column.location()))
            .cloned();
        drop(kept);
        self.returned = position.is_some();
        position
    }

    /// Remembers into `memory` from now on, the window's shared one.
    pub(super) fn share(&mut self, memory: &Memory) {
        if !self.memory.same(memory) {
            self.memory = memory.clone();
        }
    }

    /// Whether the folder shown last came back at its remembered position.
    pub(super) fn returned(&self) -> bool {
        self.returned
    }
}

/// Whether `column` finished reading its folder: listed, not loading, not failed.
fn listed(column: &Column) -> bool {
    !column.is_loading() && *column.load() == Load::Listed
}

pub(super) fn fall_back(
    column: &Column,
    collection: &mut CollectionState,
    browser_cursor: Option<EntryId>,
) {
    let selection = &collection.selection;
    if selection.cursor().is_some() {
        return;
    }
    let first = selection
        .selected_ids(column)
        .into_iter()
        .filter_map(|id| Some((column.position_of(id)?, id)))
        .min()
        .map(|(_, id)| id);
    let fallen = first.or(browser_cursor).and_then(|id| column.index_of(id));
    let (index, anchor) = match (fallen, selection.anchor()) {
        (Some(index), Some(_)) => (index, AnchorRule::Keep),
        (Some(index), None) => (index, AnchorRule::Move),
        (None, _) if column.len() > 0 => (0, AnchorRule::Keep),
        (None, _) => return,
    };
    let _ = collection.selection.move_cursor(column, index, anchor);
    collection.note_selection(column);
}

pub(super) fn take_load_cursor(column: &Column, collection: &mut CollectionState, marks: &Marks) {
    let Some(cursor) = marks.cursor.filter(|_| marks.provisional) else {
        return;
    };
    let selection = &collection.selection;
    if selection.cursor() == Some(cursor) {
        return;
    }
    let (mut selected, mut picked) = (selection.selected_ids(column), marks.selected.clone());
    selected.sort_unstable();
    picked.sort_unstable();
    let Some(index) = column.index_of(cursor).filter(|_| selected == picked) else {
        return;
    };
    let _ = collection
        .selection
        .move_cursor(column, index, AnchorRule::Move);
    collection.note_selection(column);
}

pub(super) fn keep_provisional(column: &Column, collection: &mut CollectionState, marks: &Marks) {
    let selection = &collection.selection;
    if !marks.provisional || selection.is_provisional() {
        return;
    }
    let _ = reposition(column, collection, |position| position.provisional = true);
}

pub(super) fn follow_fill(
    column: &Column,
    collection: &mut CollectionState,
    browser: &Browser,
    depth: usize,
) -> bool {
    let selection = &collection.selection;
    if !selection.is_provisional() || browser.filled_entries(depth).is_empty() {
        return false;
    }
    reposition(column, collection, |position| position.provisional = false)
}

pub(super) fn anchor_at_cursor(column: &Column, collection: &mut CollectionState) -> bool {
    let selection = &collection.selection;
    if selection.anchor().is_some() || selection.cursor().is_none() {
        return false;
    }
    reposition(column, collection, |position| {
        position.anchor = position.cursor
    })
}

fn reposition(
    column: &Column,
    collection: &mut CollectionState,
    edit: impl FnOnce(&mut Position),
) -> bool {
    let mut position = collection.selection.position(column);
    edit(&mut position);
    let changed = collection.selection.restore(&position);
    if !(changed.selection || changed.anchor) {
        return false;
    }
    collection.note_selection(column);
    true
}
