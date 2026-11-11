//! Find over the text, as Strata's source find runs (`match_ranges` in
//! `src/ui/virtual_preview`): literal and case-insensitive by lowercase folding, so "éclair"
//! finds "ÉCLAIR", with matches that don't overlap ("aa" in "aaa" is one). The folding is
//! C11's ([`Folded`]), built once per text, so typing a query scans the text without
//! rebuilding it.

use std::ops::Range;

use crate::document_view::Folded;

use super::rows::Rows;

/// The matches of one query, as byte ranges of the text, in order.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub ranges: Vec<Range<usize>>,
}

impl Found {
    pub fn new(folded: &Folded, query: &str) -> Found {
        Found {
            ranges: folded.find(query),
        }
    }

    /// The matches that touch row `r` of `rows`, cut to it, in the row's own bytes, with each
    /// one's index.
    pub fn in_row(&self, rows: &Rows, r: usize) -> impl Iterator<Item = (usize, Range<usize>)> {
        let row = rows.row(r);
        let first = self.ranges.partition_point(|m| m.end <= row.start);
        self.ranges[first..]
            .iter()
            .enumerate()
            .take_while(move |(_, m)| m.start < row.end)
            .filter_map(move |(i, m)| {
                let a = m.start.max(row.start);
                let b = m.end.min(row.end);
                (a < b).then(|| (first + i, a - row.start..b - row.start))
            })
    }
}
