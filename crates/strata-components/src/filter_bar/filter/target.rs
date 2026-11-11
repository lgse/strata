//! The Enter target's rules: Strata's `enter_target.rs` `Tracker` (`lgse/strata`
//! `upstream/main`), over C1a's selection.
//!
//! Until the user picks a result after the query last changed, Enter opens the top result,
//! and while the field has the keyboard that row is outlined. After a pick, while something is
//! selected, Enter follows Strata per mode ([`super::Shows`]): search results that replace the
//! listing open their cursor, or the top result with none (`inline_search.rs`,
//! `current_position().unwrap_or(0)`), where Down lands too; a listing narrowed in place opens
//! its first selected result in display order (`filter_activation_position`). Deselecting
//! everything gives the top result back. In 10xer mode nothing is outlined, and before any
//! results show for the text (the host's debounce, or after a restart) nothing is offered, so
//! the per-mode rules apply from the start.

use super::state::Shows;
use crate::collection::selection::Selection;
use crate::contract::{EntryId, EntrySource};

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

    /// What Enter opens: the top result while it is offered; else, for search results, the
    /// cursor or the top result; in place, the first selected result in display order. Before
    /// any results show for the text (the host's debounce, or after a restart) nothing is
    /// offered, so the per-mode rule applies at once, as Strata's.
    pub fn target(&self, r: Results<'_>, tenxer: bool, shows: Shows) -> Option<EntryId> {
        if r.source.is_empty() {
            return None;
        }
        if self.offers_top(true, !r.selection.is_empty(), tenxer) {
            return Some(r.source.id_at(0));
        }
        match shows {
            Shows::Replace => {
                let at = r.selection.cursor_index(r.source).unwrap_or(0);
                Some(r.source.id_at(at))
            }
            Shows::InPlace => first_selected(r),
        }
    }
}

/// The first selected result in display order, found without collecting the selection.
fn first_selected(r: Results<'_>) -> Option<EntryId> {
    if r.selection.is_empty() {
        return None;
    }
    (0..r.source.len())
        .find(|&i| r.selection.is_selected_at(r.source, i))
        .map(|i| r.source.id_at(i))
}
