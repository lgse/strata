// SPDX-License-Identifier: MIT

use std::ops::Range;

use rune::Span;
use strata_kit::look::Look;

#[cfg(test)]
mod tests;

/// Byte ranges of an entry's name to highlight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NameMatches {
    bytes: [u64; WORDS],
}

const WORDS: usize = 8;

impl NameMatches {
    /// The bytes of a name that can be highlighted; matches past them are dropped.
    pub const REACH: usize = WORDS * 64;

    /// `ranges`, skipping empty ones, up to [`NameMatches::REACH`].
    pub fn new(ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut matches = Self::default();
        for range in ranges {
            for byte in range.start..range.end.min(Self::REACH) {
                matches.bytes[byte / 64] |= 1 << (byte % 64);
            }
        }
        matches
    }

    /// Whether nothing is highlighted.
    pub fn is_empty(&self) -> bool {
        self.bytes.iter().all(|word| *word == 0)
    }

    fn marked(&self, byte: usize) -> bool {
        byte < Self::REACH && self.bytes[byte / 64] & (1 << (byte % 64)) != 0
    }

    /// The ranges, in order and apart.
    pub fn ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        let mut at = 0;
        std::iter::from_fn(move || {
            let start = (at..Self::REACH).find(|byte| self.marked(*byte))?;
            let end = (start..=Self::REACH)
                .find(|byte| !self.marked(*byte))
                .unwrap_or(Self::REACH);
            at = end;
            Some(start..end)
        })
    }

    /// The ranges of `name` moved onto `shown`, the name as a caption cut it.
    pub fn onto(&self, name: &str, shown: &str) -> Self {
        if name == shown {
            return *self;
        }
        let head = common_prefix(name, shown);
        let tail = common_suffix(&name[head..], &shown[head..]);
        let (tail_in_name, tail_in_shown) = (name.len() - tail, shown.len() - tail);
        let moved = |at: usize| at - tail_in_name + tail_in_shown;
        Self::new(self.ranges().flat_map(|range| {
            let front = range.start..range.end.min(head);
            let back = (range.end > tail_in_name)
                .then(|| moved(range.start.max(tail_in_name))..moved(range.end));
            [Some(front), back].into_iter().flatten()
        }))
    }

    /// The spans that paint the ranges: the accent behind, the base background in front.
    pub fn spans(&self, look: &Look) -> Vec<Span> {
        let c = &look.kit.c;
        self.ranges()
            .map(|range| {
                Span::new(range.start, range.end)
                    .color(look.transparency.opaque(c.bg_base))
                    .bg(c.bg_accent, 0.0)
            })
            .collect()
    }
}

/// The bytes `a` and `b` start with alike, on a character boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or_else(|| a.len().min(b.len()), |((at, _), _)| at)
}

/// The bytes `a` and `b` end with alike, on a character boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}
