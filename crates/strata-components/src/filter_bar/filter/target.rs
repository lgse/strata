// SPDX-License-Identifier: MIT

use super::state::Shows;
use crate::collection::selection::Selection;
use crate::contract::{EntryId, EntrySource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rules {
    pub shows: Shows,
    pub tenxer: bool,
    pub pick_last: bool,
}

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
