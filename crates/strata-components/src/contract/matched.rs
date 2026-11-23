// SPDX-License-Identifier: MIT

use std::ops::Range;

use rune::{Color, El, Span, TextStyle};
use rune_kit::Kit;

#[cfg(test)]
mod tests;

/// A label with matched ranges drawn in an accent colour.
#[derive(Clone, Debug)]
pub struct MatchedText {
    text: String,
    ranges: Vec<Range<usize>>,
    font: Option<TextStyle>,
    color: Option<Color>,
    accent: Option<Color>,
    key: Option<String>,
    truncate: bool,
}

impl MatchedText {
    /// `text` with `ranges` matched. The ranges are normalised against the text.
    pub fn new(text: impl Into<String>, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        let text = text.into();
        let ranges = normalize_ranges(&text, ranges);
        Self {
            text,
            ranges,
            font: None,
            color: None,
            accent: None,
            key: None,
            truncate: false,
        }
    }

    /// Draw the text in `font`.
    pub fn font(mut self, font: TextStyle) -> Self {
        self.font = Some(font);
        self
    }

    /// Draw the unmatched text in `color`, a Kit token.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Draw the matched ranges in `color`, a Kit token.
    pub fn accent(mut self, color: Color) -> Self {
        self.accent = Some(color);
        self
    }

    /// The element's key.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Keep to one line, cut short with an ellipsis, rather than wrap at the label's width.
    pub fn truncate(mut self, on: bool) -> Self {
        self.truncate = on;
        self
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The matched ranges, normalised.
    pub fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }

    /// The label, drawn with `k`'s tokens where no override was given.
    pub fn build(self, k: &Kit) -> El {
        let accent = self.accent.unwrap_or(k.c.text_accent);
        let spans = self
            .ranges
            .iter()
            .map(|r| Span::new(r.start, r.end).color(accent))
            .collect();
        let mut el = rune::rich(self.text, spans)
            .font(self.font.unwrap_or(k.ty.text_small))
            .color(self.color.unwrap_or(k.c.text_base));
        if self.truncate {
            el = el.truncate();
        }
        match self.key {
            Some(key) => el.key(key),
            None => el,
        }
    }
}

/// `ranges` made safe to draw over `text`: each clamped to the text.
pub fn normalize_ranges(
    text: &str,
    ranges: impl IntoIterator<Item = Range<usize>>,
) -> Vec<Range<usize>> {
    let len = text.len();
    let mut out: Vec<Range<usize>> = ranges
        .into_iter()
        .filter_map(|r| {
            let mut start = r.start.min(len);
            let mut end = r.end.min(len);
            while !text.is_char_boundary(start) {
                start -= 1;
            }
            while !text.is_char_boundary(end) {
                end += 1;
            }
            (start < end).then_some(start..end)
        })
        .collect();
    out.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The byte ranges of the characters at `char_indices` in `text`, normalised: for matchers that report character positions.
pub fn char_ranges(text: &str, char_indices: impl IntoIterator<Item = usize>) -> Vec<Range<usize>> {
    let starts: Vec<(usize, char)> = text.char_indices().collect();
    let ranges = char_indices
        .into_iter()
        .filter_map(|i| starts.get(i).map(|&(at, c)| at..at + c.len_utf8()));
    normalize_ranges(text, ranges)
}

/// A `fuzzy` match: its rank, and the matched byte ranges, normalised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Higher ranks first.
    pub score: i32,
    /// The matched characters, as byte ranges of the text.
    pub ranges: Vec<Range<usize>>,
}

/// Matches `query` against `text` as rune-kit's Combobox ranks its options, and says which characters matched.
pub fn fuzzy(query: &str, text: &str) -> Option<FuzzyMatch> {
    let needle: Vec<char> = query.trim().to_lowercase().chars().collect();
    if needle.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            ranges: Vec::new(),
        });
    }
    let (mut n, mut score) = (0, 0);
    let (mut previous, mut boundary) = (false, true);
    let mut ranges = Vec::new();
    for (at, original) in text.char_indices() {
        for c in original.to_lowercase() {
            if n < needle.len() && c == needle[n] {
                score += 10 + if previous { 12 } else { 0 } + if boundary { 8 } else { 0 };
                previous = true;
                n += 1;
                ranges.push(at..at + original.len_utf8());
            } else {
                previous = false;
                score -= 1;
            }
            boundary = !c.is_alphanumeric();
        }
    }
    (n == needle.len()).then(|| FuzzyMatch {
        score,
        ranges: normalize_ranges(text, ranges),
    })
}
