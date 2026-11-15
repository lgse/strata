// SPDX-License-Identifier: MIT

//! The Enter target's rules: Strata's `enter_target.rs` `Tracker` (`lgse/strata`
//! `upstream/main`), over C1a's selection.
//!
//! Until the user picks a result after the query last changed, Enter opens the top result,
//! and while the field has the keyboard that row is outlined. After a pick, while something is
//! selected, Enter opens the first selected result in display order (or the last when the host
//! sets [`super::FilterState::set_pick_last`], as Strata's `columns/append/keys.rs` does),
//! where Down lands too: in search results that replace a single-pane listing,
//! `inline_search/collection.rs`'s `current_position` finds no focused item widget while the
//! field has the keyboard and falls back to the first selected result (`unwrap_or(0)` with
//! none, or the unselected cursor in 10xer mode); in a listing narrowed in place,
//! `filter_activation_position` takes the first selected result and opens nothing with none
//! ([`super::Shows`]). Deselecting everything gives the top result back. In 10xer mode nothing
//! is outlined, and before any results show for the text (the host's debounce, or after a
//! restart) nothing is offered, so the per-mode rules apply from the start.

use super::state::Shows;
use crate::collection::selection::Selection;
use crate::contract::{EntryId, EntrySource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rules {
    pub shows: Shows,
    pub tenxer: bool,
    pub pick_last: bool,
}

/// What Enter in the field reads: the results the host shows under the filter and their
/// selection.
///
/// Used by: hosts, through [`super::FilterBar::results`].
#[derive(Clone, Copy)]
pub struct Results<'a> {
    /// The results, in display order: the narrowed listing or the search results.
    pub source: &'a dyn EntrySource,
    /// Their selection.
    pub selection: &'a Selection,
}

impl std::fmt::Debug for Results<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Results")
            .field("len", &self.source.len())
            .field("selected", &self.selection.len())
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tracker {
    /// The trimmed query whose results show, if any.
    pub query: Option<String>,
    /// The text was edited since the results last showed.
    pub edited: bool,
    /// The user picked a result since the query last changed.
    pub picked: bool,
    /// The user scrolled the results since the query last changed.
    pub scrolled: bool,
}

impl Tracker {
    /// Records the query whose results now show. A changed text, including one edited and
    /// typed back, drops the pick and the scroll; true when it did (outside 10xer mode).
    pub fn show_query(&mut self, query: &str, tenxer: bool) -> bool {
        let edited = std::mem::take(&mut self.edited);
        let query = query.trim();
        if query.is_empty() {
            self.clear();
            return false;
        }
        let changed = edited || self.query.as_deref() != Some(query);
        self.query = Some(query.to_owned());
        if !changed || tenxer {
            return false;
        }
        self.picked = false;
        self.scrolled = false;
        true
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Whether Enter opens the top result rather than the user's pick.
    pub fn offers_top(&self, has_rows: bool, has_selection: bool, tenxer: bool) -> bool {
        !tenxer && self.query.is_some() && has_rows && !(self.picked && has_selection)
    }

    /// Whether the results keep the top result scrolled into view.
    pub fn follows_top(&self, has_rows: bool, has_selection: bool, tenxer: bool) -> bool {
        !self.scrolled && self.offers_top(has_rows, has_selection, tenxer)
    }

    /// Where Down from the field lands, and what Enter opens in single-pane search results:
    /// the top result while it is offered, else the first selected result in display order
    /// (`inline_search/collection.rs`, `current_position` with the field focused; or the last
    /// when `rules.pick_last` is on, `columns/append/keys.rs`), else an unselected cursor (in
    /// single-pane search results) or the top result.
    pub fn landing(&self, r: Results<'_>, rules: Rules) -> Option<EntryId> {
        if r.source.is_empty() {
            return None;
        }
        if self.offers_top(true, !r.selection.is_empty(), rules.tenxer) {
            return Some(r.source.id_at(0));
        }
        selected_pick(r, rules.pick_last).or_else(|| {
            let at = if rules.pick_last {
                0
            } else {
                r.selection.cursor_index(r.source).unwrap_or(0)
            };
            Some(r.source.id_at(at))
        })
    }

    /// What Enter opens: the top result while it is offered; else the selected pick (first by
    /// default, last when `rules.pick_last` is on), falling back in single-pane search results
    /// with nothing selected to the cursor or the top result. Before any results show for the
    /// text (the host's debounce, or after a restart) nothing is offered, so the per-mode rule
    /// applies at once, as Strata's.
    pub fn target(&self, r: Results<'_>, rules: Rules) -> Option<EntryId> {
        if rules.shows == Shows::Replace && !rules.pick_last {
            return self.landing(r, rules);
        }
        if r.source.is_empty() {
            return None;
        }
        if self.offers_top(true, !r.selection.is_empty(), rules.tenxer) {
            return Some(r.source.id_at(0));
        }
        selected_pick(r, rules.pick_last)
    }
}

/// The first (or, when `last`, last) selected result in display order, found without
/// collecting the selection.
fn selected_pick(r: Results<'_>, last: bool) -> Option<EntryId> {
    if r.selection.is_empty() {
        return None;
    }
    let found = if last {
        (0..r.source.len())
            .rev()
            .find(|&i| r.selection.is_selected_at(r.source, i))
    } else {
        (0..r.source.len()).find(|&i| r.selection.is_selected_at(r.source, i))
    };
    found.map(|i| r.source.id_at(i))
}
