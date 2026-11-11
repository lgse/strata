//! The reference's search, as Strata's: a row stays when its action, its keys with their
//! context, or one of its search terms contains the query, compared in lower case with the −
//! and – that labels show read as a typed `-` (`search_folded` and `Shortcut::matches` in
//! `src/ui/shortcut_reference.rs`). No fuzzy matching: Strata has none here.

use std::ops::Range;

use super::data::{Section, Shortcut, ShortcutData};
use crate::contract::char_ranges;

/// `text` folded for the search, and for each folded byte the character of `text` it came
/// from, so a match maps back onto the shown text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Folded {
    pub text: String,
    origin: Vec<usize>,
}

/// Lowercase, with − (U+2212) and – (U+2013) as `-`.
pub(super) fn fold(text: &str) -> Folded {
    let mut out = Folded::default();
    for (i, c) in text.chars().enumerate() {
        let mut push = |f: char| {
            out.text.push(f);
            out.origin.extend(std::iter::repeat_n(i, f.len_utf8()));
        };
        match c {
            '\u{2212}' | '\u{2013}' => push('-'),
            c => c.to_lowercase().for_each(&mut push),
        }
    }
    out
}

/// The query as the search compares it: trimmed and folded.
pub(super) fn fold_query(query: &str) -> String {
    fold(query.trim()).text
}

impl Folded {
    /// The byte ranges of `shown` (the text this was folded from) where `query`, already
    /// folded, first appears; empty when it doesn't, or when the query is empty.
    pub fn ranges(&self, shown: &str, query: &str) -> Vec<Range<usize>> {
        if query.is_empty() {
            return Vec::new();
        }
        match self.text.find(query) {
            Some(at) => {
                let first = self.origin[at];
                let last = self.origin[at + query.len() - 1];
                char_ranges(shown, first..=last)
            }
            None => Vec::new(),
        }
    }

    fn contains(&self, query: &str) -> bool {
        self.text.contains(query)
    }
}

/// One row's folded text.
#[derive(Clone, Debug, Default)]
pub(super) struct RowIndex {
    pub action: Folded,
    pub before: Folded,
    pub after: Folded,
    keys: String,
    terms: Vec<String>,
}

impl RowIndex {
    fn new(row: &Shortcut) -> Self {
        let (before, after) = row
            .context
            .as_ref()
            .map_or((Folded::default(), Folded::default()), |c| {
                (fold(&c.before), fold(&c.after))
            });
        Self {
            action: fold(&row.action),
            before,
            after,
            keys: fold(&row.key_text()).text,
            terms: row.search.iter().map(|t| fold(t).text).collect(),
        }
    }

    /// Whether the row stays for `query` (folded). An empty query keeps every row.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.action.contains(query)
            || self.keys.contains(query)
            || self.terms.iter().any(|t| t.contains(query))
    }
}

/// Every row's folded text, built once per data revision.
#[derive(Clone, Debug, Default)]
pub(super) struct Index {
    pub revision: Option<u64>,
    pub sections: Vec<Vec<RowIndex>>,
}

impl Index {
    pub fn build(data: &ShortcutData) -> Self {
        Self {
            revision: Some(data.revision),
            sections: data
                .sections
                .iter()
                .map(|s| s.rows.iter().map(RowIndex::new).collect())
                .collect(),
        }
    }
}

/// A line of the results: a section's heading, or one or two of its rows side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Line {
    Heading {
        section: usize,
        shown: usize,
    },
    Rows {
        section: usize,
        first: usize,
        second: Option<usize>,
    },
}

/// The results for a query and a category, laid out in lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Lines {
    pub lines: Vec<Line>,
    /// The rows shown, and each shown row's place among them (`(section, row)` order).
    pub matches: usize,
    pub ordinal: Vec<Vec<Option<usize>>>,
}

/// The lines for `query` (folded) in `category` (a section index; `None` for All), `columns`
/// rows to a line.
pub(super) fn lines(
    sections: &[Section],
    index: &Index,
    query: &str,
    category: Option<usize>,
    columns: usize,
) -> Lines {
    let mut out = Lines {
        ordinal: sections.iter().map(|s| vec![None; s.rows.len()]).collect(),
        ..Lines::default()
    };
    for (si, rows) in index.sections.iter().enumerate() {
        if category.is_some_and(|c| c != si) {
            continue;
        }
        let shown: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.matches(query))
            .map(|(i, _)| i)
            .collect();
        if shown.is_empty() {
            continue;
        }
        out.lines.push(Line::Heading {
            section: si,
            shown: shown.len(),
        });
        for &ri in &shown {
            out.ordinal[si][ri] = Some(out.matches);
            out.matches += 1;
        }
        for pair in shown.chunks(columns.max(1)) {
            out.lines.push(Line::Rows {
                section: si,
                first: pair[0],
                second: pair.get(1).copied(),
            });
        }
    }
    out
}
