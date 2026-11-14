// SPDX-License-Identifier: MIT

//! One cursor and one selection, kept by two owners, for every view: a view's collection
//! moves them as keys and the pointer ask, and the browser holds the result, as it does for
//! the GTK views' pointer and for every key in their Icons and List modes (`SelectionSync` in
//! `src/ui/browser/columns/append/models.rs`). Shared code, owned by the contract
//! ([`crate::slots`], "Who owns what"): the Columns view uses it once for each column, a
//! single-pane view ([`crate::pane`]) once.
//!
//! - [`hand_over`] gives the browser what the collection changed, through
//!   `Browser::set_selection`. The browser answers with `SelectionSynced`, which nothing
//!   applies again.
//! - [`adopt`] writes to the collection what the browser changed by itself: the cursor a load
//!   places, the neighbour a deletion moves it to, a reveal.
//! - [`placed`] tells [`adopt`]'s one exception: a cursor the collection was waiting to place
//!   until its folder was listed is the user's.
//! - [`Moving`] carries the selection across a rename, which gives an entry another id.
//!
//! A collection's selection has a revision that moves with every change. The view keeps, for
//! each collection, the revision the browser holds ([`Held`]), so neither side hears its own
//! change back.

use strata_components::{
    collection::{
        element::CollectionState,
        selection::{CursorGone, Position, SyncRule},
    },
    contract::{EntryId, EntrySource},
};
use strata_fs::app::Browser;

use crate::{
    events::Marks,
    listing::{Column, Load},
};

/// The revision of one collection's selection that the browser holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held(u64);

impl Held {
    /// The browser holds what `collection` shows now.
    pub(crate) fn of(collection: &CollectionState) -> Self {
        Self(collection.selection.revision())
    }
}

/// How a collection follows its rows after they changed. Where a cursor goes when its entry
/// leaves is the browser's to say: the view only lets go of it.
pub(crate) fn sync_rule(column: &Column) -> SyncRule {
    match column.load() {
        Load::Loading => SyncRule::loading(),
        Load::Listed | Load::Failed(_) => SyncRule::complete(CursorGone::Clear),
    }
}

/// Whether the sync that just ran placed a cursor the collection had none of before
/// (`before`): one it was waiting to place until the folder was listed, as Right into a column
/// still loading leaves it. That cursor is the user's, so the browser's marks must not
/// replace it: [`hand_over`] gives it to the browser instead.
pub(crate) fn placed(before: Option<EntryId>, collection: &CollectionState) -> bool {
    before.is_none() && collection.selection.cursor().is_some()
}

/// Writes the browser's cursor and selection, `marks`, to the collection of `column`, unless
/// it shows them already, with the browser's Shift anchor, so a range the user is extending
/// goes on from where it started. Returns the revision the browser then holds.
pub(crate) fn adopt(column: &Column, collection: &mut CollectionState, marks: &Marks) -> Held {
    // A folder still loading may not list an entry the browser already holds.
    let loading = *column.load() == Load::Loading;
    let listed = |id: &EntryId| loading || column.index_of(*id).is_some();
    let cursor = marks.cursor.filter(listed);
    let selected: Vec<EntryId> = marks.selected.iter().copied().filter(listed).collect();
    let provisional = marks.provisional && cursor.is_some_and(|id| selected == [id]);
    let selection = &mut collection.selection;
    // The browser's anchor, as GTK extends a Shift range from it after a reload; the cursor
    // when it has none.
    let anchor = marks.anchor.filter(listed).or(cursor);
    let shown = selection.cursor() == cursor
        && selection.is_provisional() == provisional
        && selection.len() == selected.len()
        && selected.iter().all(|id| selection.contains(column, *id));
    if !shown {
        let _ = selection.restore(&Position {
            cursor,
            cursor_index: cursor.and_then(|id| column.index_of(id)),
            anchor,
            selected,
            provisional,
            scroll: None,
        });
        collection.note_selection(column);
    }
    Held::of(collection)
}

/// One of the browser's columns as a view shows it: the listing, and the collection over it.
pub(crate) struct Shown<'a> {
    pub(crate) depth: usize,
    pub(crate) column: &'a Column,
    pub(crate) collection: &'a CollectionState,
}

/// What a view holds of the browser's columns, as the QA backend's invariants read it
/// (`qa/invariants.rs`): borrowed from the view, which keeps nothing for it.
#[cfg(feature = "qa")]
#[derive(Default)]
pub(crate) struct Presented<'a> {
    /// The browser's column the view keeps the keyboard's cursor in: Columns' active column, a
    /// single pane's shown one.
    pub(crate) active: Option<usize>,
    /// For a view that draws a column for each of the browser's, Columns: how many it draws.
    /// None for a view that shows one folder.
    pub(crate) strip: Option<usize>,
    /// The listing the view keeps for each of the browser's columns, root first.
    pub(crate) listings: Vec<&'a Column>,
    /// The columns shown with a cursor and a selection.
    pub(crate) shown: Vec<Shown<'a>>,
}

/// Gives the browser's column what its collection changed since the browser last held it.
/// Call it before GLib dispatches and before any call that makes the browser act on its
/// cursor, so the browser never works from a selection older than the view's.
pub(crate) fn hand_over(shown: &Shown<'_>, browser: &Browser, held: &mut Held) {
    let Shown {
        depth,
        column,
        collection,
    } = *shown;
    let selection = &collection.selection;
    let now = Held::of(collection);
    if *held == now {
        return;
    }
    *held = now;
    let position = |id: EntryId| column.position_of(id);
    let selected: Vec<usize> = selection
        .selected_ids(column)
        .into_iter()
        .filter_map(position)
        .collect();
    let cursor = selection.cursor().and_then(position);
    tracing::trace!(
        depth,
        selected = selected.len(),
        ?cursor,
        "the browser takes the view's selection"
    );
    // The user's own choice, as a key or a click is to the GTK views: it replaces a cursor a
    // load placed.
    browser.commit_selection();
    browser.set_selection(depth, &selected, cursor);
    if let Some(anchor) = selection.anchor().and_then(position) {
        browser.set_selection_anchor(depth, anchor);
    }
}

/// Entries one of the browser's operations gave another id (a rename gives an entry another
/// location), each as its old id and its new one, until the listing shows the change. The
/// change comes before the listing shows it, and a selection may only be told once its listing
/// has changed.
#[derive(Debug, Default)]
pub(crate) struct Moving(Vec<(EntryId, EntryId)>);

impl Moving {
    /// The entry `from` has the id `to` now. `lists` says whether the view lists an id. A
    /// listing that changed first, as when the folder's monitor reported the rename before the
    /// operation did, has let go of the old id already: nothing waits then.
    pub(crate) fn moved(&mut self, from: EntryId, to: EntryId, lists: impl Fn(EntryId) -> bool) {
        if lists(from) && !lists(to) {
            self.0.push((from, to));
        }
    }

    /// `column`'s rows changed, and `collection`, its selection, has not been brought in step
    /// with them yet. What it holds by the old id of an entry that now shows under its new one
    /// it holds by the new one: a range keeps its anchor and its members through a rename.
    /// Returns the pairs that arrived, for what else the view holds by id.
    pub(crate) fn arrived(
        &self,
        column: &Column,
        collection: &mut CollectionState,
    ) -> Vec<(EntryId, EntryId)> {
        let arrived = self.arriving(column);
        for (from, to) in &arrived {
            let _ = collection.selection.relocate(*from, *to);
        }
        arrived
    }

    /// The pairs whose entry `column` now shows under its new id, and no longer under its old.
    pub(crate) fn arriving(&self, column: &Column) -> Vec<(EntryId, EntryId)> {
        let here = |(from, to): &(EntryId, EntryId)| {
            column.position_of(*to).is_some() && column.position_of(*from).is_none()
        };
        self.0.iter().copied().filter(here).collect()
    }

    /// Drops the pairs whose old id `listed` no longer finds: they have nothing left to move.
    pub(crate) fn forget(&mut self, listed: impl Fn(EntryId) -> bool) {
        self.0.retain(|(from, _)| listed(*from));
    }
}
