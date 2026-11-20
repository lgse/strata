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
//!   browser's own cursor, so the two never disagree; with neither, to the first row shown,
//!   as a first visit places it ([`fall_back`]). A remembered cursor whose entry went lets go
//!   of it for this, rather than take the entry at its index.
//! - **No choice.** What comes back is handed to the browser without committing it, as GTK's
//!   `restore_selection` sets the browser's selection alone: a cursor a load placed (Alt+Up's
//!   folder come from) stays the load's, so Columns shown again opens no child for it
//!   (`follow_selection_on_return`; finding lgse/strata#2437). Where the browser's selection is
//!   such a cursor and what came back is the cursor alone, the view's pick stays provisional
//!   too ([`keep_provisional`]), as the browser's does. Where what came back selects exactly
//!   the load's pick with its cursor elsewhere, the browser keeps the load's cursor, and the
//!   view takes it ([`take_load_cursor`]), as GTK's view draws it. Where the browser holds no
//!   load cursor, what is handed back is a fill there, and the view's pick is no longer a
//!   load's either ([`follow_fill`]).
//! - **One memory for the window's single-pane views** ([`Memory`]), as GTK's
//!   `PaneNavigation` is one history whatever the mode: a folder left in Icons comes back in
//!   List where Icons left it. A view switched away hears nothing, so while the view shown
//!   again is rebuilt from the browser what it showed last is stale and is not remembered
//!   (finding lgse/strata#2425).
//! - **The cursor a first visit placed** ([`Memory::visit_cursor`]), shared the same way. GTK's
//!   browser holds no cursor after a first visit: the ring on the first row is the list
//!   widget's own focus (`grouped_focus_drifted` in `src/ui/browser_modes/events.rs` asks for
//!   no cursor), so each GTK view rebuilt over it, by a regroup or a view switch, rings its own
//!   first row shown, and the browser takes a cursor only when the user moves one. The Rune
//!   side must hold one for F3's `browser-view` invariant, which keeps the view's cursor the
//!   browser's, so the first visit hands its cursor over and the memory keeps it as the visit's
//!   until the user's own input moves the cursor or the selection. While it does, a single pane
//!   that shows it off its first row moves it there, with nothing selected
//!   (`single::first_visit`; finding lgse/strata#2427).

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

/// Where each folder was left, shared by the window's single-pane views: the shell holds it,
/// and each view's [`super::Pane`] is given it as a change applies.
#[derive(Clone, Default)]
pub(crate) struct Memory(Rc<RefCell<Kept>>);

#[derive(Default)]
struct Kept {
    places: RestoreMemory<Location>,
    /// A view switched to is being rebuilt from the browser: the folder it leaves is stale.
    replaying: bool,
    /// The cursor a first visit placed, in the browser's column at that depth, which no input
    /// has moved since.
    visit: Option<(usize, EntryId)>,
    /// The rows the folder showed when that cursor was last placed or kept: how many, and the
    /// first. A view rebuilt over the same rows (a regroup, a view switch) shows its own first
    /// row, as GTK's does, but an entry added or gone is no rebuild, and GTK's list keeps its
    /// focus on the same row.
    visit_rows: (usize, Option<EntryId>),
}

impl Memory {
    /// While `replaying`, a view leaving a folder does not remember it.
    pub(crate) fn set_replaying(&self, replaying: bool) {
        self.0.borrow_mut().replaying = replaying;
    }

    /// The cursor a first visit placed and no input has moved since: the browser's column and
    /// the entry.
    pub(crate) fn visit_cursor(&self) -> Option<(usize, EntryId)> {
        self.0.borrow().visit
    }

    /// The cursor a first visit placed is at `at` now, or there is none.
    pub(crate) fn set_visit_cursor(&self, at: Option<(usize, EntryId)>) {
        self.0.borrow_mut().visit = at;
    }

    /// The rows the folder showed when the visit's cursor was last placed or kept: how many,
    /// and the first.
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
    let fallen = first.or(browser_cursor).and_then(|id| column.index_of(id));
    let (index, anchor) = match (fallen, selection.anchor()) {
        (Some(index), Some(_)) => (index, AnchorRule::Keep),
        (Some(index), None) => (index, AnchorRule::Move),
        // Nothing to fall back to, as after a folder opened by a click: GTK's list view,
        // taking the keyboard with no cursor, focuses its first row, as on a first visit
        // (`single::first_visit`), with nothing selected and no anchor.
        (None, _) if column.len() > 0 => (0, AnchorRule::Keep),
        (None, _) => return,
    };
    let _ = collection.selection.move_cursor(column, index, anchor);
    collection.note_selection(column);
}

/// After a restore landed over the browser's `marks`: where what came back selects exactly the
/// entries the browser's load picked, the browser reads it as its own pick and keeps the load's
/// cursor (`classify_reported_fill`'s echo), so the view takes that cursor too. GTK's view
/// draws it there, not on the remembered row, and its rename and Delete act on it (driven on
/// GTK, 2026-11-17). Nothing is committed, so Columns shown again opens no child for it
/// (finding lgse/strata#2437).
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

/// After a restore landed over the browser's `marks`: where the browser's selection is the
/// cursor a load placed, the browser keeps it the load's through the restore, which commits
/// nothing, so a pick of the cursor alone stays provisional in the view as it does there.
pub(super) fn keep_provisional(column: &Column, collection: &mut CollectionState, marks: &Marks) {
    let selection = &collection.selection;
    if !marks.provisional || selection.is_provisional() {
        return;
    }
    let _ = reposition(column, collection, |position| position.provisional = true);
}

/// After a remembered position was handed back as no choice: the browser keeps a cursor a load
/// placed through it, but where it holds none, as after a folder opened by a double click, the
/// position is a fill there, whatever the view remembered of it. So a pick the view brought
/// back as a load's is the user's in the view too, and the two agree on what the next toggle
/// and a 10xer verb act on (seed 0x62 of `list v3` and 0x3e of `icons v1`: a folder first
/// opened by Return, opened again by a double click). Returns whether the selection changed.
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

/// After a remembered position was handed back: one with a cursor and no anchor takes the
/// cursor as its anchor, as the browser's `set_selection` gives it, which GTK's
/// `restore_selection` goes through, so a Shift+click after a revisit ranges from the cursor
/// there too (`pointer v4` seed `0x2f`). Returns whether the selection changed.
pub(super) fn anchor_at_cursor(column: &Column, collection: &mut CollectionState) -> bool {
    let selection = &collection.selection;
    if selection.anchor().is_some() || selection.cursor().is_none() {
        return false;
    }
    reposition(column, collection, |position| {
        position.anchor = position.cursor
    })
}

/// Restores the collection's own position with `edit` made to it. Returns whether the selection
/// or its anchor changed.
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
