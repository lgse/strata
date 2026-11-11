//! Find over a document's text, as Strata's rendered preview finds (`match_ranges` and
//! `find_matches` in `src/ui/virtual_preview`): literal and case-insensitive by lowercase
//! folding, so "éclair" finds "ÉCLAIR", matches that don't overlap ("aa" in "aaa" is one), and
//! one text across units, joined by a newline where a block or line ends, so a match can
//! span two units. Table cells, images, rules and gaps aren't searched.
//!
//! The joined, folded text is built once per document ([`Index`]); each query only searches
//! it, so typing into the find bar costs a scan of the document, not a rebuild of it.

use std::ops::Range;

/// A point in the document: a unit and a byte offset into its shown text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub unit: usize,
    pub offset: usize,
}

/// A match, from `start` to `end` (exclusive), which may lie in different units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Match {
    pub start: Point,
    pub end: Point,
}

/// A text folded to lowercase, with the way back to the text's own offsets.
#[derive(Debug, Default)]
pub(crate) struct Folded {
    pub folded: String,
    /// `(folded offset, text offset)` wherever the two stop moving together: around each
    /// character whose lowercase takes a different number of bytes. Between marks, an
    /// offset in one is the same distance into the other.
    marks: Vec<(usize, usize)>,
    len: usize,
}

impl Folded {
    pub fn new(text: &str) -> Folded {
        let mut folded = String::with_capacity(text.len());
        let mut marks = vec![(0, 0)];
        for (b, ch) in text.char_indices() {
            let before = folded.len();
            if ch.is_ascii() {
                folded.push(ch.to_ascii_lowercase());
                continue;
            }
            folded.extend(ch.to_lowercase());
            if folded.len() - before != ch.len_utf8() {
                marks.push((before, b));
                marks.push((folded.len(), b + ch.len_utf8()));
            }
        }
        Folded {
            folded,
            marks,
            len: text.len(),
        }
    }

    /// The text offset of folded offset `at`: inside a character that folded to another
    /// length, its start.
    fn back(&self, at: usize) -> usize {
        let i = self.marks.partition_point(|&(f, _)| f <= at) - 1;
        let (f, t) = self.marks[i];
        // Past a mark that opens a resized character, everything up to its closing mark is
        // that character: map it to its start.
        if self
            .marks
            .get(i + 1)
            .is_some_and(|&(nf, nt)| nf > at && nf - f != nt - t)
        {
            return t;
        }
        (t + (at - f)).min(self.len)
    }

    /// The text offset of folded offset `at` taken as a match's end: inside a resized
    /// character, its end.
    fn back_end(&self, at: usize) -> usize {
        let i = self.marks.partition_point(|&(f, _)| f < at);
        if let Some(&(nf, nt)) = self.marks.get(i)
            && i > 0
        {
            let (f, t) = self.marks[i - 1];
            if nf - f != nt - t {
                return nt;
            }
        }
        self.back(at)
    }

    /// Non-overlapping matches of `query` (folded too), as ranges of the original text.
    pub fn find(&self, query: &str) -> Vec<Range<usize>> {
        if query.is_empty() {
            return Vec::new();
        }
        let needle: String = query.chars().flat_map(char::to_lowercase).collect();
        self.folded
            .match_indices(needle.as_str())
            .map(|(start, m)| self.back(start)..self.back_end(start + m.len()))
            .collect()
    }
}

/// Non-overlapping, case-insensitive matches of `query` in `text`, as byte ranges.
#[cfg(test)]
pub(crate) fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    Folded::new(text).find(query)
}

/// The searchable text of a document, joined and folded once.
#[derive(Debug, Default)]
pub(crate) struct Index {
    text: Folded,
    /// Where each unit's text starts in the joined text, and the unit.
    starts: Vec<(usize, usize)>,
    lens: Vec<usize>,
}

impl Index {
    /// The searchable texts in order, each a unit, its shown text, and whether a newline
    /// follows it in the text find reads (the unit ends its block or its line).
    pub fn new<'a>(texts: impl IntoIterator<Item = (usize, &'a str, bool)>) -> Index {
        let mut joined = String::new();
        let mut starts = Vec::new();
        let mut lens = Vec::new();
        for (unit, text, newline) in texts {
            if text.is_empty() {
                continue;
            }
            starts.push((joined.len(), unit));
            lens.push(text.len());
            joined.push_str(text);
            if newline && !text.ends_with('\n') {
                joined.push('\n');
            }
        }
        Index {
            text: Folded::new(&joined),
            starts,
            lens,
        }
    }

    pub fn find(&self, query: &str) -> Vec<Match> {
        if self.starts.is_empty() {
            return Vec::new();
        }
        let point = |offset: usize| {
            let i = self
                .starts
                .partition_point(|&(start, _)| start <= offset)
                .saturating_sub(1);
            let (start, unit) = self.starts[i];
            Point {
                unit,
                offset: (offset - start).min(self.lens[i]),
            }
        };
        self.text
            .find(query)
            .into_iter()
            .map(|r| Match {
                start: point(r.start),
                end: point(r.end),
            })
            .collect()
    }
}
