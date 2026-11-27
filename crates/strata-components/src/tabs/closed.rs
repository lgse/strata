// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

/// How many closed tabs a window keeps: a 21st close drops the oldest.
pub const CLOSED_TABS_KEPT: usize = 20;

/// A closed tab: the host's record of it (its location), and where it stood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed<T> {
    pub item: T,
    pub index: usize,
}

impl<T> Closed<T> {
    pub fn index_in(&self, len: usize) -> usize {
        self.index.min(len)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTabs<T> {
    entries: VecDeque<Closed<T>>,
    kept: usize,
}

impl<T> Default for ClosedTabs<T> {
    fn default() -> Self {
        ClosedTabs::new()
    }
}

impl<T> ClosedTabs<T> {
    /// An empty stack keeping [`CLOSED_TABS_KEPT`].
    pub fn new() -> Self {
        ClosedTabs::keeping(CLOSED_TABS_KEPT)
    }

    /// An empty stack keeping `kept` (at least one).
    pub fn keeping(kept: usize) -> Self {
        ClosedTabs {
            entries: VecDeque::new(),
            kept: kept.max(1),
        }
    }

    /// Records `item`, closed at `index`, dropping the oldest past the limit.
    pub fn record(&mut self, item: T, index: usize) {
        if self.entries.len() == self.kept {
            self.entries.pop_front();
        }
        self.entries.push_back(Closed { item, index });
    }

    pub fn reopen(&mut self) -> Option<Closed<T>> {
        self.entries.pop_back()
    }

    /// The newest closed tab, left in place.
    pub fn peek(&self) -> Option<&Closed<T>> {
        self.entries.back()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forgets every closed tab.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
