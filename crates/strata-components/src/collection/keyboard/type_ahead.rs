//! Type-ahead: finding an entry by the prefix typed so far. Time arrives as data.

use std::time::Duration;

use crate::contract::EntrySource;

/// The prefix typed so far, and when its last character came.
#[derive(Clone, Debug, Default)]
pub(super) struct TypeAhead {
    /// The prefix, lowercased.
    typed: String,
    last: f64,
}

impl TypeAhead {
    /// Forgets the prefix: a key the map binds, or a folder change (the machine's `reset`).
    pub(super) fn reset(&mut self) {
        self.typed.clear();
    }

    /// Whether a prefix is still open at `now`.
    pub(super) fn open(&self, now: f64, timeout: Duration) -> bool {
        !self.typed.is_empty() && now - self.last <= timeout.as_secs_f64()
    }

    /// Adds `c` at `now` and finds the entry to move to, from the cursor at `cursor`.
    ///
    /// A character after the timeout starts a new prefix. A new prefix, or the same letter
    /// typed again ("a", "a", "a"), finds the next match after the cursor, so repeating a
    /// letter steps through the entries that start with it. A longer prefix finds the first
    /// match from the cursor itself, so typing on keeps the entry it already found. The search
    /// wraps past the end. Names compare case-insensitively.
    pub(super) fn push(
        &mut self,
        c: char,
        now: f64,
        timeout: Duration,
        source: &dyn EntrySource,
        cursor: Option<usize>,
    ) -> Option<usize> {
        if !self.open(now, timeout) {
            self.typed.clear();
        }
        self.last = now;
        self.typed.extend(c.to_lowercase());
        let len = source.len();
        if len == 0 {
            return None;
        }
        let repeat = repeats_one(&self.typed);
        let (query, start) = match cursor {
            Some(i) if self.typed.chars().count() > 1 && !repeat => (self.typed.as_str(), i),
            Some(i) => (first_char(&self.typed), (i + 1) % len),
            None => (self.typed.as_str(), 0),
        };
        (0..len)
            .map(|k| (start + k) % len)
            .find(|&i| starts_with_folded(source.entry(i).name, query))
    }
}

/// Whether `s` is one character typed more than once ("aaa").
fn repeats_one(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let mut more = false;
    for c in chars {
        if c != first {
            return false;
        }
        more = true;
    }
    more
}

fn first_char(s: &str) -> &str {
    s.char_indices().nth(1).map_or(s, |(i, _)| &s[..i])
}

/// Whether `name`, lowercased, starts with `query`, which is lowercase already. Allocates
/// nothing, so a search over 100,000 names stays cheap.
fn starts_with_folded(name: &str, query: &str) -> bool {
    let mut name = name.chars().flat_map(char::to_lowercase);
    query.chars().all(|q| name.next() == Some(q))
}
