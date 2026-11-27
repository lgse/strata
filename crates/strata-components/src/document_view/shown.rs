// SPDX-License-Identifier: MIT

use std::ops::Range;

use super::model::{Span, SpanStyle};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Insert {
    pub old: Range<usize>,
    pub new: String,
    pub copy: String,
}

/// A text as drawn: the shown text, its spans, and where its inserts sit in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shown {
    pub text: String,
    pub spans: Vec<Span>,
    /// Each insert's range in `text`, with what copying it takes. Sorted, not overlapping.
    pub runs: Vec<(Range<usize>, String)>,
    edits: Vec<(Range<usize>, usize)>,
}

impl Shown {
    pub fn with(text: &str, spans: &[Span], mut inserts: Vec<Insert>) -> Shown {
        inserts.sort_by_key(|i| (i.old.start, i.old.end));
        let mut out = String::with_capacity(text.len());
        let mut runs = Vec::new();
        let mut edits = Vec::new();
        let mut at = 0;
        for ins in &inserts {
            out.push_str(&text[at..ins.old.start]);
            let start = out.len();
            out.push_str(&ins.new);
            runs.push((start..out.len(), ins.copy.clone()));
            edits.push((ins.old.clone(), ins.new.len()));
            at = ins.old.end;
        }
        out.push_str(&text[at..]);
        let mut shown = Shown {
            text: out,
            spans: Vec::new(),
            runs,
            edits,
        };
        shown.spans = spans
            .iter()
            .map(|s| Span {
                range: shown.map(s.range.start)..shown.map(s.range.end),
                style: s.style.clone(),
            })
            .collect();
        shown
    }

    pub fn map(&self, at: usize) -> usize {
        let mut shift: isize = 0;
        for (old, new_len) in &self.edits {
            let past = if old.is_empty() {
                at > old.start
            } else {
                at >= old.end
            };
            if past {
                shift += *new_len as isize - old.len() as isize;
            }
        }
        (at as isize + shift) as usize
    }

    pub fn map_ranges<T: Clone>(&self, ranges: &[(Range<usize>, T)]) -> Vec<(Range<usize>, T)> {
        if self.edits.is_empty() {
            return ranges.to_vec();
        }
        ranges
            .iter()
            .map(|(r, t)| (self.map(r.start)..self.map(r.end), t.clone()))
            .collect()
    }

    pub fn copy(&self, range: Range<usize>) -> String {
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        let mut out = String::new();
        let mut at = range.start;
        for (run, copy) in &self.runs {
            if run.end <= range.start || run.start >= range.end {
                continue;
            }
            if at < run.start {
                out.push_str(&self.text[at..run.start]);
            }
            out.push_str(copy);
            at = run.end.max(at);
        }
        if at < range.end {
            out.push_str(&self.text[at..range.end]);
        }
        out
    }
}

/// The inserts for a unit's inline equations: each placeholder becomes its source.
pub(crate) fn math_inserts(text: &str, spans: &[Span]) -> Vec<Insert> {
    spans
        .iter()
        .filter_map(|s| match &s.style {
            SpanStyle::Math(source) if s.range.end <= text.len() => Some(Insert {
                old: s.range.clone(),
                new: source.to_string(),
                copy: format!("${source}$"),
            }),
            _ => None,
        })
        .collect()
}
