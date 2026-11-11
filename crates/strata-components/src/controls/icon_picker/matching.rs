//! Search: GTK's emoji chooser rule (`gtk/gtkemojichooser.c`, `match_tokens`, GTK 4.18).
//!
//! The query and the name are split into words (runs of letters and digits) and folded
//! (lowercased, Latin accents dropped, so "cafe" finds "café"). An item matches when every
//! query word starts one of its name's words, or else when every query word starts one of its
//! keywords (each keyword is matched whole, as GTK matches them). Matches keep the catalog's
//! order: there is no ranking.

use std::borrow::Cow;
use std::ops::Range;

/// An item's search words, worked out once when the source is built.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Words {
    /// The name's words, folded, each with its byte range in the name.
    name: Vec<(String, Range<usize>)>,
    /// The keywords, folded.
    keywords: Vec<String>,
}

/// What a query matched in an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    /// The name: for each query word, the name word it starts and how many characters it
    /// covers there.
    Name(Vec<(usize, usize)>),
    /// A keyword, the first the first query word starts, and how many characters that word
    /// covers there.
    Keyword(usize, usize),
}

/// A query: its folded words. A query with no words matches nothing ([`Query::is_empty`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Query {
    words: Vec<String>,
}

impl Query {
    pub(crate) fn new(text: &str) -> Self {
        Self {
            words: split(text).map(|(w, _)| fold(w)).collect(),
        }
    }

    /// No words: the query is blank (or only punctuation).
    pub(crate) fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// What this query matches in `words`, if anything.
    pub(crate) fn hit(&self, words: &Words) -> Option<Hit> {
        if self.words.is_empty() {
            return None;
        }
        let on_name: Option<Vec<(usize, usize)>> = self
            .words
            .iter()
            .map(|q| {
                words
                    .name
                    .iter()
                    .position(|(w, _)| w.starts_with(q.as_str()))
                    .map(|i| (i, q.chars().count()))
            })
            .collect();
        if let Some(found) = on_name {
            return Some(Hit::Name(found));
        }
        let all = self
            .words
            .iter()
            .all(|q| words.keywords.iter().any(|k| k.starts_with(q.as_str())));
        if !all {
            return None;
        }
        let first = &self.words[0];
        words
            .keywords
            .iter()
            .position(|k| k.starts_with(first.as_str()))
            .map(|i| Hit::Keyword(i, first.chars().count()))
    }
}

/// The search words of an item called `name` with `keywords`.
pub(crate) fn words(name: &str, keywords: &[Cow<'static, str>]) -> Words {
    Words {
        name: split(name).map(|(w, r)| (fold(w), r)).collect(),
        keywords: keywords.iter().map(|k| fold(k)).collect(),
    }
}

/// The byte ranges of `name` a name hit covers: each query word's share of the start of the
/// name word it matched. Empty for a keyword hit.
pub(crate) fn name_ranges(name: &str, words: &Words, hit: &Hit) -> Vec<Range<usize>> {
    let Hit::Name(found) = hit else {
        return Vec::new();
    };
    found
        .iter()
        .filter_map(|&(word, chars)| {
            let (_, range) = words.name.get(word)?;
            let part = name.get(range.clone())?;
            // Folding keeps a character a character here, but a lowercase that grows ("İ")
            // would count more: the cut stays inside the word either way.
            let end = part
                .char_indices()
                .nth(chars)
                .map_or(range.end, |(at, _)| range.start + at);
            Some(range.start..end)
        })
        .collect()
}

/// The byte range of `keyword` a keyword hit lights: the first query word's share of its start,
/// as [`name_ranges`] lights a name word.
pub(crate) fn keyword_range(keyword: &str, chars: usize) -> Range<usize> {
    0..keyword
        .char_indices()
        .nth(chars)
        .map_or(keyword.len(), |(at, _)| at)
}

/// The words of `text`: runs of letters and digits, with their byte ranges.
fn split(text: &str) -> impl Iterator<Item = (&str, Range<usize>)> {
    let mut start = None;
    let mut out = Vec::new();
    for (at, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(at),
            (false, Some(s)) => {
                out.push((&text[s..at], s..at));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((&text[s..], s..text.len()));
    }
    out.into_iter()
}

/// `text` lowercased, with the accents of Latin letters dropped.
pub(crate) fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(base_letter)
        .collect()
}

/// A lowercase Latin letter with an accent as its base letter; anything else as it is.
fn base_letter(c: char) -> char {
    const TABLE: &[(&str, char)] = &[
        ("àáâãäåāăą", 'a'),
        ("çćĉċč", 'c'),
        ("ďđ", 'd'),
        ("èéêëēĕėęě", 'e'),
        ("ĝğġģ", 'g'),
        ("ĥħ", 'h'),
        ("ìíîïĩīĭįı", 'i'),
        ("ĵ", 'j'),
        ("ķ", 'k'),
        ("ĺļľŀł", 'l'),
        ("ñńņňŉ", 'n'),
        ("òóôõöøōŏő", 'o'),
        ("ŕŗř", 'r'),
        ("śŝşš", 's'),
        ("ţťŧ", 't'),
        ("ùúûüũūŭůűų", 'u'),
        ("ŵ", 'w'),
        ("ýÿŷ", 'y'),
        ("źżž", 'z'),
    ];
    if c.is_ascii() {
        return c;
    }
    TABLE
        .iter()
        .find(|(from, _)| from.contains(c))
        .map_or(c, |&(_, to)| to)
}
