//! What a unit's text element shows, when it isn't the model's text as it is: an inline
//! equation's placeholder replaced by its source (D2), and the room the Copy code button
//! keeps at the end of a code block's first line. Each change is an [`Insert`], which knows
//! what copying it takes, so a selection copies Strata's text whatever is drawn.

use std::ops::Range;

use super::model::{Span, SpanStyle};

/// A run the view draws in place of `old` (a range of the model's text, empty for a pure
/// insertion), and what copying any of it takes.
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
    /// The inserts by old range, to move offsets of the model's text (C10a's colours).
    edits: Vec<(Range<usize>, usize)>,
}

impl Shown {
    /// `text` with `inserts` (sorted by `old`, not overlapping) applied, and `spans` moved to
    /// match. An inline equation's span covers its source, drawn in the mono face.
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

    /// Where byte `at` of the model's text is in the shown text. An offset at a replacement's
    /// end lands after it; an offset at a pure insertion stays before it, so a span ending
    /// at the line's end doesn't reach over the padding.
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

    /// `ranges` of the model's text (C10a's colours), in the shown text.
    pub fn map_ranges<T: Clone>(&self, ranges: &[(Range<usize>, T)]) -> Vec<(Range<usize>, T)> {
        if self.edits.is_empty() {
            return ranges.to_vec();
        }
        ranges
            .iter()
            .map(|(r, t)| (self.map(r.start)..self.map(r.end), t.clone()))
            .collect()
    }

    /// What copying bytes `range` of the shown text takes: the text, with each insert it
    /// touches replaced by the insert's copy (an equation's `$source$`, nothing for padding).
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
