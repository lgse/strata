// SPDX-License-Identifier: MIT

//! Where a folder was left, brought back when it shows again: GTK's `PaneNavigation`
//! (`src/ui/browser_modes/navigation.rs`), on C1e's position restore.
//!
//! - **Remembered on leaving** a folder that had finished loading (GTK's `loaded_location`):
//!   its cursor, anchor and selection by entry id, and its scroll offset in the view's layout,
//!   for the 128 folders left most recently (C1e's [`RestoreMemory`], GTK's `HISTORY_LIMIT`).
//! - **Recalled when it shows again loading,** by any navigation, as GTK prepares a restore
//!   whenever a folder in its history starts loading (`prepare` returns at once for a
//!   snapshot not loading): a column already listed that shows again (a deeper one closed)
//!   keeps what the browser holds. C1e brings the selection back once the listing
//!   is complete, and the offset only in the layout it was measured in (GTK: the same mode,
//!   and in Icons the same page width); otherwise it reveals the cursor. Input in the
//!   collection before then cancels it, as GTK's `arm_input_cancel` does.
//! - **An explicit reveal wins** (`reveal_pending`, as GTK's `prepare` cancels for it): the
//!   remembered position stays for a later visit.
//! - **By id.** A view index holds only against the order the listing has now (a grouped
//!   folder shows in its groups' order); C1e's [`Position`] keeps ids, so the restore finds
//!   the same entries whatever order they show in.
//! - **A cursor falls back** to the first restored selected entry when its own entry is gone,
//!   or none was remembered, and the anchor to the cursor; with no selection either, to the
//!   browser's own cursor, so the two never disagree ([`fall_back`]).
//! - **One memory for the window's single-pane views** ([`Memory`]), as GTK's
//!   `PaneNavigation` is one history whatever the mode: a folder left in Icons comes back in
//!   List where Icons left it. A view switched away hears nothing, so while the view shown
//!   again is rebuilt from the browser what it showed last is stale and is not remembered
//!   (finding lgse/strata#2425).

use std::{cell::RefCell, rc::Rc};

use strata_components::{
    collection::{
        element::CollectionState,
        selection::{AnchorRule, Position, RestoreMemory},
    },
    contract::{EntryId, EntrySource},
};
use strata_core::model::Location;

use crate::listing::{Column, Load};

/// Where each folder was left, shared by the window's single-pane views: the shell holds it,
/// and each view's [`super::Pane`] is given it as a change applies.
#[derive(Clone, Default)]
pub(crate) struct Memory(Rc<RefCell<Kept>>);

#[derive(Default)]
struct Kept {
    places: RestoreMemory<Location>,
    /// A view switched to is being rebuilt from the browser: the folder it leaves is stale.
    replaying: bool,
}

impl Memory {
    /// While `replaying`, a view leaving a folder does not remember it.
    pub(crate) fn set_replaying(&self, replaying: bool) {
        self.0.borrow_mut().replaying = replaying;
    }

    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
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
    /// A change that may show another folder starts: the folder shown now, if it was listed,
    /// is where `collection` stands. A folder still loading or whose read failed is not
    /// remembered, as GTK's `loaded_location` skips one with an error.
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

    /// Another folder shows, `column` or none: the folder left is remembered, and the
    /// position remembered for `column`, unless the browser reveals an entry there.
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

/// After a restore landed: a cursor whose entry is gone, or none remembered, falls back to
/// the first restored selected entry in the browser's order, and the anchor to the cursor
/// where it has none, as GTK's `restore_selection` does (`focused.or_else(|| positions.first())`).
/// With no selection either, the cursor is the browser's own, `browser_cursor` (the folder
/// come from, on Back): the browser keeps it whatever the view hands over, and the view must
/// never show a cursor other than the entry Return or Delete act on. None of it scrolls: the
/// restored offset stays.
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
    let Some(index) = first.or(browser_cursor).and_then(|id| column.index_of(id)) else {
        return;
    };
    let anchor = match selection.anchor() {
        Some(_) => AnchorRule::Keep,
        None => AnchorRule::Move,
    };
    let _ = collection.selection.move_cursor(column, index, anchor);
    collection.note_selection(column);
}
