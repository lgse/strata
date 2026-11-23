// SPDX-License-Identifier: MIT

//! [`FilterState`]: the filter bar's state, held by the host and reached by accessor.

use rune::TextEdit;

use super::target::{Results, Rules, Tracker};
use crate::collection::selection::Position;
use crate::contract::{EntryId, Point};
use crate::filter_bar::field::FocusWatch;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shows {
    #[default]
    Replace,
    InPlace,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FilterIntent {
    Query {
        /// The field's text.
        text: String,
    },
    ShowListing {
        /// The selection to put back, if any.
        restore: Option<Position>,
    },
    /// The filter closed (Escape, or [`FilterState::close`]).
    Closed,
    Open {
        /// The Enter target.
        id: EntryId,
    },
    FocusResults {
        /// The result Down lands on.
        id: EntryId,
    },
    Preview,
    FieldMenu {
        /// Where the menu opens, in window points.
        at: Point,
    },
}

/// What a call into the bar changed, and its intents for the host.
#[must_use = "route the intents as the bar's own"]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polled {
    /// The bar's look changed: the host's poll returns true.
    pub changed: bool,
    /// The intents, in order.
    pub intents: Vec<FilterIntent>,
}

impl Polled {
    pub(super) fn changed() -> Self {
        Self {
            changed: true,
            intents: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, intent: FilterIntent) {
        self.changed = true;
        self.intents.push(intent);
    }

    pub(super) fn merge(&mut self, other: Polled) {
        self.changed |= other.changed;
        self.intents.extend(other.intents);
    }
}

#[derive(Debug, Default)]
pub struct FilterState {
    pub edit: TextEdit,
    open: bool,
    shows: Shows,
    tenxer: bool,
    pick_last: bool,
    pub(crate) tracker: Tracker,
    /// The listing's selection from before the filter, while results replace it.
    saved: Option<Position>,
    /// The results changed since the last [`FilterState::take_reveal`].
    reveal: bool,
    watch: FocusWatch,
    /// Whether the field had the keyboard at the last tick, for the outline.
    was_focused: bool,
}

impl FilterState {
    /// Closed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the bar shows.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn field_focused(&self) -> bool {
        self.open && self.edit.focused
    }

    /// The field's text.
    pub fn text(&self) -> &str {
        &self.edit.text
    }

    /// How the host shows the results.
    pub fn shows(&self) -> Shows {
        self.shows
    }

    /// Whether 10xer mode's rules apply: no outline, and a query edit keeps the selection.
    pub fn tenxer(&self) -> bool {
        self.tenxer
    }

    pub fn set_shows(&mut self, shows: Shows) {
        self.shows = shows;
    }

    /// Turns 10xer mode's rules on or off.
    pub fn set_tenxer(&mut self, on: bool) {
        self.tenxer = on;
    }

    pub fn pick_last(&self) -> bool {
        self.pick_last
    }

    pub fn set_pick_last(&mut self, on: bool) {
        self.pick_last = on;
    }

    fn rules(&self) -> Rules {
        Rules {
            shows: self.shows,
            tenxer: self.tenxer,
            pick_last: self.pick_last,
        }
    }

    pub fn open(&mut self) -> Polled {
        if self.open {
            self.focus_field();
            return Polled::changed();
        }
        self.open = true;
        self.edit = TextEdit::new("");
        self.edit.focused = true;
        self.watch.reset();
        self.tracker.clear();
        self.saved = None;
        Polled::changed()
    }

    pub fn open_with(&mut self, text: &str, before: Position) -> Polled {
        let mut out = self.open();
        self.edit = TextEdit::new(text);
        self.edit.set_caret(text.len());
        self.edit.focused = true;
        out.merge(self.text_changed(|| before));
        out
    }

    pub fn focus_field(&mut self) {
        if !self.open {
            return;
        }
        let end = self.edit.text.len();
        self.edit.set_caret(end);
        self.edit.focused = true;
    }

    pub fn close(&mut self) -> Polled {
        if !self.open {
            return Polled::default();
        }
        let mut out = Polled::changed();
        if !self.edit.text.trim().is_empty() {
            out.push(FilterIntent::ShowListing {
                restore: self.saved.take(),
            });
        }
        self.open = false;
        self.edit = TextEdit::new("");
        self.tracker.clear();
        self.saved = None;
        self.reveal = false;
        self.watch.reset();
        self.was_focused = false;
        out.push(FilterIntent::Closed);
        out
    }

    pub fn shown(&mut self, query: &str) -> bool {
        self.reveal = true;
        let changed = self.tracker.show_query(query, self.tenxer);
        changed && self.shows == Shows::Replace
    }

    pub fn pick(&mut self) {
        if self.open {
            self.tracker.picked = true;
        }
    }

    pub fn restart(&mut self) {
        self.tracker.query = None;
    }

    pub fn results_scrolled(&mut self) {
        self.tracker.scrolled = true;
    }

    pub fn target(&self, r: Results<'_>) -> Option<EntryId> {
        if !self.open || self.edit.text.trim().is_empty() {
            return None;
        }
        self.tracker.target(r, self.rules())
    }

    pub fn landing(&self, r: Results<'_>) -> Option<EntryId> {
        self.tracker.landing(r, self.rules())
    }

    pub fn outlined(&self, r: Results<'_>) -> Option<EntryId> {
        if !self.field_focused() || r.source.is_empty() {
            return None;
        }
        self.tracker
            .offers_top(true, !r.selection.is_empty(), self.tenxer)
            .then(|| r.source.id_at(0))
    }

    pub fn take_reveal(&mut self, r: Results<'_>) -> Option<usize> {
        if !std::mem::take(&mut self.reveal) || !self.field_focused() {
            return None;
        }
        let rows = !r.source.is_empty();
        self.tracker
            .follows_top(rows, !r.selection.is_empty(), self.tenxer)
            .then_some(0)
    }

    pub fn tick(&mut self) -> Polled {
        if !self.open {
            return Polled::default();
        }
        let focused = self.edit.focused;
        self.watch.lost(focused);
        let changed = focused != self.was_focused;
        self.was_focused = focused;
        Polled {
            changed,
            intents: Vec::new(),
        }
    }

    pub fn text_edited(&mut self, before: Position) -> Polled {
        self.text_changed(|| before)
    }

    // The bar's own handlers.

    pub(super) fn needs_save(&self) -> bool {
        self.shows == Shows::Replace && self.saved.is_none() && !self.edit.text.trim().is_empty()
    }

    pub(crate) fn text_changed(&mut self, before: impl FnOnce() -> Position) -> Polled {
        let mut out = Polled::changed();
        self.tracker.edited = true;
        self.reveal = true;
        let text = self.edit.text.clone();
        if text.trim().is_empty() {
            self.tracker.clear();
            out.push(FilterIntent::ShowListing {
                restore: self.saved.take(),
            });
            return out;
        }
        if self.shows == Shows::Replace && self.saved.is_none() {
            self.saved = Some(before());
        }
        out.push(FilterIntent::Query { text });
        out
    }
}
