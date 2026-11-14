// SPDX-License-Identifier: MIT

//! One cursor and one selection for each column, kept by two owners through
//! [`crate::handover`], which every view shares: [`Columns::publish`] hands the browser what
//! a column's collection changed, and [`Columns::adopt`] writes to it what the browser changed
//! by itself. What is the Columns view's own is the active column and the mirror that follows
//! the cursor.

use std::rc::Rc;

use strata_components::{
    collection::selection::AnchorRule,
    contract::{EntryId, EntrySource},
};
use strata_fs::app::Browser;

use super::Columns;
use crate::handover::{self, Held, Shown};

/// What made the view change its cursor or its selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Keys {
        /// Shift was held: the key grew or shrank a range.
        extending: bool,
        /// The view reported the cursor for this key, which it also does when the key left
        /// the cursor where it was (Home on the first entry, an arrow at the list's end). The
        /// browser raises `FocusChanged` again for such a key, so the child column follows
        /// the cursor whether it moved or not.
        rested: bool,
    },
    Pointer,
}
use crate::events::Marks;

impl Columns {
    pub(super) fn cursor(&self, depth: usize) -> Option<EntryId> {
        self.state.column(depth)?.collection.selection.cursor()
    }

    /// What the browser holds of column `depth` once it has its collection's selection.
    pub(super) fn held_now(&self, depth: usize) -> Held {
        self.state
            .column(depth)
            .map_or_else(Held::default, |column| Held::of(&column.collection))
    }

    /// The active column and its cursor: what the GTK views call the focused item.
    pub(super) fn focused(&self) -> Option<(usize, EntryId)> {
        let depth = self.state.active();
        (depth < self.open())
            .then(|| self.cursor(depth))
            .flatten()
            .map(|id| (depth, id))
    }

    /// Writes the browser's cursor and selection for column `depth` to the view, unless the
    /// view shows them already ([`handover::adopt`]). Not while a filter's results show
    /// there: their selection is the filter's, which the browser does not hold.
    pub(super) fn adopt(&mut self, depth: usize, marks: &Marks) {
        if self.listings.hits_at(depth) {
            return;
        }
        let (Some(column), Some(state)) = (
            self.listings.columns.get(depth),
            self.state.column_mut(depth),
        ) else {
            return;
        };
        let now = handover::adopt(column, &mut state.collection, marks);
        if let Some(held) = self.held.get_mut(depth) {
            *held = now;
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

    /// Not while a filter's results show in column `depth`: GTK's hits only tell the preview
    /// and the footer (`search_hits_changed`), never the browser.
    fn hand_over(&mut self, depth: usize, browser: &Browser) {
        if self.listings.hits_at(depth) {
            return;
        }
        let (Some(column), Some(state), Some(held)) = (
            self.listings.columns.get(depth),
            self.state.column(depth),
            self.held.get_mut(depth),
        ) else {
            return;
        };
        let shown = Shown {
            depth,
            column,
            collection: &state.collection,
        };
        handover::hand_over(&shown, browser, held);
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
        let again = matches!(input, Some(Input::Keys { rested: true, .. }));
        let Some((depth, id)) = focused.filter(|_| again || focused != before) else {
            return;
        };
        // A filter's result opens no column under it.
        if self.listings.hits_at(depth) {
            return;
        }
        // Shift says a range moved. Without it (a key fed in with no modifier held), a range
        // still shows: its anchor stays behind the cursor, in the column the cursor was in.
        let same_column = before.is_some_and(|(earlier, _)| earlier == depth);
        let anchored_behind = same_column
            && self
                .state
                .column(depth)
                .is_some_and(|column| column.collection.selection.anchor() != Some(id));
        let extending = matches!(
            input,
            Some(Input::Keys {
                extending: true,
                ..
            })
        );
        let ranging = extending || anchored_behind;
        if let Some(position) = self.position(depth, id).filter(|_| !ranging) {
            self.mirror.follow(browser, depth, position, false);
        }
    }

    /// Selects every row of column `depth`, as the menu's Select All does: Ctrl+A in the
    /// Columns view, whose cursor goes to the last entry (`select_all` in
    /// `src/ui/browser/view_state.rs`, through GTK's selection model).
    pub(crate) fn select_all(&mut self, depth: usize) {
        let (Some(column), Some(state)) =
            (self.listings.shown(depth), self.state.column_mut(depth))
        else {
            return;
        };
        let Some(last) = column.len().checked_sub(1) else {
            return;
        };
        let selection = &mut state.collection.selection;
        let anchor = match selection.anchor() {
            Some(_) => AnchorRule::Keep,
            None => AnchorRule::Move,
        };
        selection.select_all(column);
        selection.move_cursor(column, last, anchor);
        state.collection.note_selection(column);
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
