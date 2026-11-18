// SPDX-License-Identifier: MIT

//! The parts of an entry's name the host says match what the user looks for, drawn over the
//! name in the accent (Strata's find and filter highlights, `highlight_attributes` in
//! `src/ui/browser/find.rs`). Which parts match is the host's: the component only paints them.

use std::ops::Range;

use rune::Span;
use strata_kit::look::Look;

#[cfg(test)]
mod tests;

/// Byte ranges of an entry's name to highlight, as many as the name holds within its first
/// [`NameMatches::REACH`] bytes (a file name's own limit is 255): one bit for each byte, so it
/// stays `Copy` and rides in [`RowMarks`](super::RowMarks), and a change of ranges builds the
/// row again. Ranges that touch read back as one, which paints the same.
///
/// Used by: hosts (Strata's 10xer find and filter), C1e's label, C3's caption, C4's name cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NameMatches {
    bytes: [u64; WORDS],
}

const WORDS: usize = 8;

impl NameMatches {
    /// The bytes of a name that can be highlighted; matches past them are dropped. A range across
    /// it is cut there, which may fall inside a character of a longer name: the host, which
    /// knows the name, ends its ranges on a character boundary within it.
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

    /// The ranges of `name` moved onto `shown`, the name as a caption cut it: the parts in the
    /// ends `shown` keeps of `name` stay highlighted, and the parts the cut dropped go.
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

    /// The spans that paint the ranges: the accent behind, the base background in front, as
    /// Strata's solid highlight stays visible over a selected row's tint.
    pub fn spans(&self, look: &Look) -> Vec<Span> {
        let c = &look.kit.c;
        self.ranges()
            .map(|range| {
                Span::new(range.start, range.end)
                    .color(c.bg_base)
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
