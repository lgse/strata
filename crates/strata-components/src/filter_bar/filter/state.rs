//! [`FilterState`]: the filter bar's state, held by the host and reached by accessor.

use rune::TextEdit;

use super::target::{Results, Tracker};
use crate::collection::selection::Position;
use crate::contract::{EntryId, Point};
use crate::filter_bar::field::FocusWatch;

/// How the host shows the filter's results.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shows {
    /// Search results replace the listing (a local folder): they start with no selection, a
    /// query edit drops the pick and their selection, and closing the filter gives the
    /// listing back the selection it had before the filter ([`FilterIntent::ShowListing`]).
    #[default]
    Replace,
    /// The filter narrows the listing's own rows (Trash, Recent, a network share): the
    /// listing's selection stays through query edits and after the filter closes.
    InPlace,
}

/// What the filter bar asks of its host. Filtering, opening and previewing are the host's.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FilterIntent {
    /// The query changed to `text` (raw: spaces kept): filter the source, then call
    /// [`FilterState::shown`] with it.
    Query {
        /// The field's text.
        text: String,
    },
    /// The query is empty (cleared, or the filter closed): show the whole listing. With
    /// results that replace the listing, `restore` is the listing's selection from before
    /// the filter: show the listing, then `Selection::restore` it, then `sync`.
    ShowListing {
        /// The selection to put back, if any.
        restore: Option<Position>,
    },
    /// The filter closed (Escape, or [`FilterState::close`]).
    Closed,
    /// Enter in the field: open entry `id` ([`FilterState::target`]), file or folder,
    /// whatever the click and preview preferences.
    Open {
        /// The Enter target.
        id: EntryId,
    },
    /// Down in the field: put the cursor on `id` (selecting it, as Strata's Down does) and
    /// give the results the keyboard. The bar then moves the keyboard to the results
    /// ([`super::FilterBar::tab_to_results`]). It is the pick from now on.
    FocusResults {
        /// The result Down lands on.
        id: EntryId,
    },
    /// Space in the field with a result selected that the host's `previews` accepts: toggle
    /// its quick preview. The query stays.
    Preview,
    /// A right-click, the Menu key or Shift+F10 on the field: the host's field menu (C21's
    /// `FieldMenu::open`), at `at` in window points. Never the folder menu.
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

/// The filter bar's state: whether it is open, its text, the Enter target's tracking, the
/// listing's selection from before the filter, and whether the top result follows the
/// results. The host keeps one per pane and hands the bar an accessor to it.
///
/// The host calls, besides the bar's own handlers:
///
/// - [`FilterState::open`] (Ctrl+F, `/`) or [`FilterState::open_with`] (type to search);
/// - [`FilterState::shown`] once the results for a query show, and [`FilterState::take_reveal`]
///   after it;
/// - [`FilterState::pick`] when the user changes the results' selection or gives them the
///   keyboard, and [`FilterState::results_scrolled`] on a press, wheel or drag over them;
/// - [`FilterState::focus_field`] for Up from the first result, or Ctrl+F with the filter open;
/// - [`FilterState::close`] for Escape from a result;
/// - [`FilterState::tick`] each frame, which notices the field losing the keyboard.
#[derive(Debug, Default)]
pub struct FilterState {
    /// The field's text, caret and focus. The bar binds it; a host reads it, and C21's
    /// `FieldMenu` edits it.
    pub edit: TextEdit,
    open: bool,
    shows: Shows,
    tenxer: bool,
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

    /// Whether the field has the keyboard: the host's key dispatch then leaves the keys the
    /// field keeps alone (Ctrl+A edits the query; Ctrl+1–3 switch the view and keep it).
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

    /// Sets how the host shows the results (per location: in place at Trash, Recent and
    /// network shares).
    pub fn set_shows(&mut self, shows: Shows) {
        self.shows = shows;
    }

    /// Turns 10xer mode's rules on or off.
    pub fn set_tenxer(&mut self, on: bool) {
        self.tenxer = on;
    }

    /// Opens the bar empty with the keyboard in it: Ctrl+F, the pane's filter button, `/`
    /// with Type to search on. Already open, it keeps its query and takes the keyboard back,
    /// the caret after the query.
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

    /// Opens the bar on `text` with the keyboard in it, the caret after it: a printable key
    /// typed in the listing with Type to search on. Already open (the key came from a result),
    /// `text` replaces the query, as Strata's `show_filter_with_query` does. `before` is the
    /// listing's selection now (`Selection::position`), which closing the filter gives back.
    /// Asks for the query.
    pub fn open_with(&mut self, text: &str, before: Position) -> Polled {
        let mut out = self.open();
        self.edit = TextEdit::new(text);
        self.edit.set_caret(text.len());
        self.edit.focused = true;
        out.merge(self.text_changed(|| before));
        out
    }

    /// Gives the field the keyboard back, the caret after the query: Up from the first result,
    /// or Ctrl+F (`/`) with the filter open. Rune moves focus on the next frame (probe P2).
    pub fn focus_field(&mut self) {
        if !self.open {
            return;
        }
        let end = self.edit.text.len();
        self.edit.set_caret(end);
        self.edit.focused = true;
    }

    /// Closes the bar and shows the whole listing: Escape in the field, Escape from a result,
    /// or the host (Columns' click outside the filtered column). Nothing when closed.
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

    /// The results for `query` now show (the host filtered for a [`FilterIntent::Query`], or
    /// a same-query update arrived). True when the text changed since the last call, edited
    /// and typed back included: with results that replace the listing, the host then clears
    /// their selection, so the outline moves to the new top result. A same-query update keeps
    /// the pick, the selection and its preview. In 10xer mode it never returns true.
    pub fn shown(&mut self, query: &str) -> bool {
        self.reveal = true;
        let changed = self.tracker.show_query(query, self.tenxer);
        changed && self.shows == Shows::Replace
    }

    /// The user picked a result: a selection change of theirs (a click, Ctrl+click, an arrow,
    /// a marquee) or the keyboard moving into the results. Enter follows the selection from
    /// now on, and the outline goes, until the query changes or nothing is selected.
    pub fn pick(&mut self) {
        if self.open {
            self.tracker.picked = true;
        }
    }

    /// Treats the next shown query as new, after the host dropped the results it showed (a
    /// scope or a search limit changed, Strata's `EnterTarget::restart`): the next
    /// [`FilterState::shown`] drops the pick even for the same text.
    pub fn restart(&mut self) {
        self.tracker.query = None;
    }

    /// A press, wheel, touch or drag hover on the results: they stop keeping the top result
    /// in view until the next query edit.
    pub fn results_scrolled(&mut self) {
        self.tracker.scrolled = true;
    }

    /// What Enter in the field opens: the top result until the user picks one after the query
    /// last changed; after that, per mode, as Strata does. Search results that replace the
    /// listing ([`Shows::Replace`]) open their cursor, or the top result with none
    /// (`inline_search.rs`, `current_position`): where Down lands. A listing narrowed in place
    /// ([`Shows::InPlace`]) opens its first selected result in display order
    /// (`filter_activation_position`). `None` with an empty query, no results, or (in place,
    /// after a pick or in 10xer mode) nothing selected.
    pub fn target(&self, r: Results<'_>) -> Option<EntryId> {
        if !self.open || self.edit.text.trim().is_empty() {
            return None;
        }
        self.tracker.target(r, self.tenxer, self.shows)
    }

    /// Where Down from the field lands: the top result while Enter opens it, else the results'
    /// cursor (Strata's search results, `inline_search.rs`), else the top result. `None` with
    /// no results.
    pub fn landing(&self, r: Results<'_>) -> Option<EntryId> {
        if r.source.is_empty() {
            return None;
        }
        let top = self
            .tracker
            .offers_top(true, !r.selection.is_empty(), self.tenxer);
        let cursor = r.selection.cursor_index(r.source).filter(|_| !top);
        Some(r.source.id_at(cursor.unwrap_or(0)))
    }

    /// The result to outline (C1e's `RowMarks::enter_target`): the top result while Enter
    /// opens it and the field has the keyboard.
    pub fn outlined(&self, r: Results<'_>) -> Option<EntryId> {
        if !self.field_focused() || r.source.is_empty() {
            return None;
        }
        self.tracker
            .offers_top(true, !r.selection.is_empty(), self.tenxer)
            .then(|| r.source.id_at(0))
    }

    /// The index to reveal (`CollectionState::reveal`) after results arrived, once each time:
    /// the top result, while the field has the keyboard and the user hasn't scrolled them
    /// since the query changed.
    pub fn take_reveal(&mut self, r: Results<'_>) -> Option<usize> {
        if !std::mem::take(&mut self.reveal) || !self.field_focused() {
            return None;
        }
        let rows = !r.source.is_empty();
        self.tracker
            .follows_top(rows, !r.selection.is_empty(), self.tenxer)
            .then_some(0)
    }

    /// One frame: notices the field gaining or losing the keyboard (Rune draft U-C6-3), which
    /// shows or hides the outline. The filter stays open when the keyboard leaves it.
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

    /// The field's text changed outside the field's own editing: C21's `FieldMenu` cut,
    /// pasted or deleted (`FieldMenu::on_change`). The same as typing: asks for the query, or
    /// gives the listing back when it emptied. `before` is the listing's selection now, saved
    /// if a query starts.
    pub fn text_edited(&mut self, before: Position) -> Polled {
        self.text_changed(|| before)
    }

    // The bar's own handlers.

    /// Whether the next edit would save the listing's selection: a query starting over
    /// results that replace the listing.
    pub(super) fn needs_save(&self) -> bool {
        self.shows == Shows::Replace && self.saved.is_none() && !self.edit.text.trim().is_empty()
    }

    /// The field's text changed (typed, pasted, cut or deleted). `before` reads the listing's
    /// selection, saved when a query starts over results that replace the listing.
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
