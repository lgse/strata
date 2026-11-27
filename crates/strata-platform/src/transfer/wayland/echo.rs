// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

#[derive(Debug, Default)]
pub(super) struct Echoes {
    waiting: VecDeque<(u64, bool)>,
    /// The newest writes judged dropped, so a cancel can be told from a loss.
    dropped: VecDeque<u64>,
}

/// How many dropped writes [`Echoes::was_dropped`] remembers.
const KEEP_DROPPED: usize = 64;

impl Echoes {
    /// A write by `source` went out with a sync after it.
    pub(super) fn arm(&mut self, source: u64) {
        self.waiting.push_back((source, false));
    }

    /// The data device heard a `selection` event: the oldest write waiting has its answer.
    pub(super) fn selection(&mut self) {
        if let Some((_, seen)) = self.waiting.front_mut() {
            *seen = true;
        }
    }

    pub(super) fn done(&mut self, source: u64) -> bool {
        let dropped = match self.waiting.iter().position(|(s, _)| *s == source) {
            Some(i) => self.waiting.remove(i).is_some_and(|(_, seen)| !seen),
            None => false,
        };
        if dropped {
            self.dropped.push_back(source);
            if self.dropped.len() > KEEP_DROPPED {
                self.dropped.pop_front();
            }
        }
        dropped
    }

    /// Whether `source`'s write was judged dropped (among the newest ones).
    pub(super) fn was_dropped(&self, source: u64) -> bool {
        self.dropped.contains(&source)
    }
}

#[cfg(test)]
mod tests;
