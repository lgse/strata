// SPDX-License-Identifier: MIT

//! One cursor and one selection, kept by two owners. The view moves them as keys and the
//! pointer ask, and the browser holds the result, as it does for the GTK views' pointer and
//! for every key in their Icons and List modes (`SelectionSync` in
//! `src/ui/browser/columns/append/models.rs`).
//!
//! - [`Columns::publish`] hands the browser what the view changed, through
//!   `Browser::set_selection`. The browser answers with `SelectionSynced`, which nothing
//!   applies again.
//! - [`Columns::adopt`] writes to the view what the browser changed by itself: the cursor a
//!   load places, the neighbour a deletion moves it to, a reveal.
//!
//! The view's selection has a revision that moves with every change. The adapter remembers,
//! for each column, the revision the browser holds, so neither side hears its own change
//! back.

use std::rc::Rc;

use strata_components::{
    collection::selection::Position,
    contract::{EntryId, EntrySource},
};
use strata_fs::app::Browser;

use super::{Columns, listing::Load};

/// What made the view change its cursor or its selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Keys {
        /// Shift was held: the key grew or shrank a range.
        extending: bool,
    },
    Pointer,
}
use crate::events::Marks;

impl Columns {
    pub(super) fn cursor(&self, depth: usize) -> Option<EntryId> {
        self.state.column(depth)?.collection.selection.cursor()
    }

    pub(super) fn revision(&self, depth: usize) -> u64 {
        self.state
            .column(depth)
            .map_or(0, |column| column.collection.selection.revision())
    }

    /// The active column and its cursor: what the GTK views call the focused item.
    fn focused(&self) -> Option<(usize, EntryId)> {
        let depth = self.state.active();
        (depth < self.open())
            .then(|| self.cursor(depth))
            .flatten()
            .map(|id| (depth, id))
    }

    /// Writes the browser's cursor and selection for column `depth` to the view, unless the
    /// view shows them already: a write ends a Shift range the user is extending.
    pub(super) fn adopt(&mut self, depth: usize, marks: &Marks) {
        let (Some(column), Some(state)) = (
            self.listings.columns.get(depth),
            self.state.column_mut(depth),
        ) else {
            return;
        };
        // A folder still loading may not list an entry the browser already holds.
        let loading = *column.load() == Load::Loading;
        let listed = |id: &EntryId| loading || column.index_of(*id).is_some();
        let cursor = marks.cursor.filter(listed);
        let selected: Vec<EntryId> = marks.selected.iter().copied().filter(listed).collect();
        let provisional = marks.provisional && cursor.is_some_and(|id| selected == [id]);
        let selection = &mut state.collection.selection;
        let shown = selection.cursor() == cursor
            && selection.is_provisional() == provisional
            && selection.len() == selected.len()
            && selected.iter().all(|id| selection.contains(column, *id));
        if !shown {
            selection.restore(&Position {
                cursor,
                cursor_index: cursor.and_then(|id| column.index_of(id)),
                anchor: cursor,
                selected,
                provisional,
                scroll: None,
            });
            state.collection.note_selection(column);
        }
        if let Some(held) = self.held.get_mut(depth) {
            *held = state.collection.selection.revision();
        }
    }

    /// Hands the browser what the view changed since it last agreed with it, and lets the
    /// child column follow a cursor that moved. Call it before GLib dispatches and before
    /// any call that makes the browser act on its cursor, so the browser never works from a
    /// selection older than the view's. `input` is what made the change, where the caller
    /// knows: the view says nothing of a key that moves between columns.
    pub(crate) fn publish(&mut self, browser: &Rc<Browser>, input: Option<Input>) {
        let open = self.open();
        for depth in 0..open {
            self.hand_over(depth, browser);
        }
        let active = self.state.active();
        if active < open && browser.active_depth() != Some(active) {
            browser.set_active_column(active);
        }
        self.follow(browser, input);
    }

    fn hand_over(&mut self, depth: usize, browser: &Browser) {
        let (Some(column), Some(state)) =
            (self.listings.columns.get(depth), self.state.column(depth))
        else {
            return;
        };
        let selection = &state.collection.selection;
        let revision = selection.revision();
        let Some(held) = self.held.get_mut(depth).filter(|held| **held != revision) else {
            return;
        };
        *held = revision;
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
        // The user's own choice, as a key or a click is to the GTK views: it replaces a
        // cursor a load placed.
        browser.commit_selection();
        browser.set_selection(depth, &selected, cursor);
        if let Some(anchor) = selection.anchor().and_then(position) {
            browser.set_selection_anchor(depth, anchor);
        }
    }

    /// Tells the mirror when the focused item moved since it last looked. The GTK views
    /// mirror from `FocusChanged`, which a cursor the view moved does not raise, so the
    /// adapter says so here. The browser raises it for a move to another row or column, and
    /// not for a Shift range growing or shrinking (`Browser::extend_selection`).
    fn follow(&mut self, browser: &Rc<Browser>, input: Option<Input>) {
        if let Some(input) = input {
            self.mirror.note_input(matches!(input, Input::Keys { .. }));
        }
        let focused = self.focused();
        let before = std::mem::replace(&mut self.followed, focused);
        let Some((depth, id)) = focused.filter(|_| focused != before) else {
            return;
        };
        // Shift says a range moved. Without it (a key fed in with no modifier held), a range
        // still shows: its anchor stays behind the cursor, in the column the cursor was in.
        let same_column = before.is_some_and(|(earlier, _)| earlier == depth);
        let anchored_behind = same_column
            && self
                .state
                .column(depth)
                .is_some_and(|column| column.collection.selection.anchor() != Some(id));
        let ranging = matches!(input, Some(Input::Keys { extending: true })) || anchored_behind;
        if let Some(position) = self.position(depth, id).filter(|_| !ranging) {
            self.mirror.follow(browser, depth, position, false);
        }
    }

    /// The pointer is over the view at `at`, in window points.
    pub(crate) fn pointer_at(&self, at: (f32, f32)) {
        self.mirror.note_pointer(at);
    }

    /// The browser moved the cursor itself, and said so in an event that mirrors it or does
    /// not: [`Columns::publish`] must not take the move for the view's.
    pub(crate) fn caught_up(&mut self) {
        self.followed = self.focused();
    }
}
