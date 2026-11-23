// SPDX-License-Identifier: MIT

use std::rc::Rc;

use strata_components::{
    collection::{keyboard::KeyHandle, selection::AnchorRule},
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

    pub(super) fn focused(&self) -> Option<(usize, EntryId)> {
        let depth = self.state.active();
        (depth < self.open())
            .then(|| self.cursor(depth))
            .flatten()
            .map(|id| (depth, id))
    }

    pub(super) fn kept_neighbour(
        &self,
        depth: usize,
        before: Option<EntryId>,
        marks: &Marks,
    ) -> Option<Marks> {
        if self.listings.hits_at(depth) {
            return None;
        }
        let column = self.listings.columns.get(depth)?;
        let state = self.state.column(depth)?;
        handover::kept_neighbour(before, column, &state.collection, marks)
    }

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
        let kept = self
            .kept
            .get_mut(depth)
            .and_then(|kept| kept.marks(&state.collection, marks));
        let marks = kept.as_ref().unwrap_or(marks);
        let now = handover::adopt(column, &mut state.collection, marks);
        if let Some(held) = self.held.get_mut(depth) {
            *held = now;
        }
    }

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
        let ranging = if self.tenxer.get() {
            self.state
                .column(depth)
                .is_some_and(|column| column.key_handle().range().is_some())
        } else {
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
            extending || anchored_behind
        };
        if let Some(position) = self.position(depth, id).filter(|_| !ranging) {
            self.mirror.follow(browser, depth, position, false);
        }
    }

    pub(crate) fn active_depth(&self) -> usize {
        self.state.active()
    }

    pub(crate) fn active_key_handle(&self) -> Option<KeyHandle> {
        let depth = self.state.active();
        self.state
            .column(depth)
            .map(|column| column.key_handle().clone())
    }

    pub(crate) fn end_ranges(&self) -> bool {
        let mut ended = false;
        for depth in 0..self.state.len() {
            if let Some(column) = self.state.column(depth) {
                ended |= column.key_handle().end_range();
            }
        }
        ended
    }

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

    pub(crate) fn end_gestures(&mut self) {
        self.state.reset();
        for depth in 0..self.state.len() {
            if let Some(column) = self.state.column_mut(depth) {
                column.collection.end_gestures();
            }
        }
    }

    pub(crate) fn caught_up(&mut self) {
        self.followed = self.focused();
    }
}
