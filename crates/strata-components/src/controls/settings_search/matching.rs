// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::ops::Range;

use super::index::SearchIndex;

/// Strata's `normalized`: lowercase, words split at non-alphanumerics, joined by one space.
pub fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The longest query Strata searches, in characters.
pub const MAX_QUERY_CHARS: usize = 128;

fn distance(a: &str, b: &str) -> usize {
    let b = b.chars().collect::<Vec<_>>();
    let mut previous = (0..=b.len()).collect::<Vec<_>>();
    let mut next = Vec::with_capacity(b.len() + 1);
    for (i, a) in a.chars().enumerate() {
        next.clear();
        next.push(i + 1);
        for (j, b) in b.iter().enumerate() {
            next.push(
                (previous[j + 1] + 1)
                    .min(next[j] + 1)
                    .min(previous[j] + usize::from(a != *b)),
            );
        }
        std::mem::swap(&mut previous, &mut next);
    }
    previous[b.len()]
}

/// Kana and Han ideographs.
fn is_unspaced_script(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{3134f}'
    )
}

/// Chinese and Japanese queries have no word boundaries for a prefix to anchor on.
fn is_unspaced_query(query: &str) -> bool {
    query.chars().nth(1).is_some() && query.chars().any(is_unspaced_script)
}

fn is_near_miss(query: &str, word: &str) -> bool {
    let allowed = match query.chars().count() {
        0..=3 => return false,
        4..=5 => 1,
        _ => 2,
    };
    // Each edit changes the length by at most one.
    let (q, w) = (query.chars().count(), word.chars().count());
    q.abs_diff(w) <= allowed && distance(query, word) <= allowed
}

fn word_score(query: &str, word: &str) -> Option<i32> {
    if query == word {
        Some(80)
    } else if word.starts_with(query) {
        Some(60)
    } else if is_unspaced_query(query) && word.contains(query) {
        Some(40)
    } else if is_near_miss(query, word) {
        Some(20)
    } else {
        None
    }
}

/// A row, prepared once per index revision.
#[derive(Clone, Debug)]
struct Candidate {
    /// The normalised English and shown titles, empty for a row without a title.
    titles: [String; 2],
    title_words: Vec<String>,
    keyword_words: Vec<String>,
}

impl Candidate {
    fn new(source: Option<&str>, shown: Option<&str>, keywords: &str) -> Self {
        let shown = shown.map(normalized).unwrap_or_default();
        let source = source.map(normalized).unwrap_or_else(|| shown.clone());
        let joined: String = shown.split_whitespace().collect();
        let mut title_words: Vec<String> = source
            .split_whitespace()
            .chain(shown.split_whitespace())
            .map(str::to_owned)
            .collect();
        if !joined.is_empty() {
            title_words.push(joined);
        }
        Self {
            titles: [source, shown],
            title_words,
            keyword_words: normalized(keywords)
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        }
    }

    /// Titles count in full; keywords count half.
    fn word(&self, query: &str) -> Option<i32> {
        let title = self
            .title_words
            .iter()
            .filter_map(|w| word_score(query, w))
            .max();
        let keyword = self
            .keyword_words
            .iter()
            .filter_map(|w| word_score(query, w).map(|s| s / 2))
            .max();
        title.max(keyword)
    }

    fn title_bonus(&self, query: &str) -> i32 {
        if self.titles[0].is_empty() {
            0
        } else if self.titles.iter().any(|t| t == query) {
            1000
        } else if self.titles.iter().any(|t| t.contains(query)) {
            200
        } else {
            0
        }
    }

    fn score(&self, query: &str) -> Option<i32> {
        let mut total = 0;
        for word in query.split_whitespace() {
            total += self.word(word)?;
        }
        Some(total + self.title_bonus(query))
    }
}

/// The index, prepared for matching.
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub revision: u64,
    rows: Vec<Candidate>,
    /// Each row's page index, `None` for a page the index doesn't list.
    row_page: Vec<Option<usize>>,
    row_group: Vec<Option<usize>>,
    pub row_by_key: HashMap<String, usize>,
    pub page_by_key: HashMap<String, usize>,
}

impl Prepared {
    pub fn new(index: &SearchIndex) -> Self {
        let page_by_key: HashMap<String, usize> = index
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| (p.key.clone(), i))
            .collect();
        let row_by_key: HashMap<String, usize> = index
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.key.clone(), i))
            .collect();
        let row_page: Vec<Option<usize>> = index
            .rows
            .iter()
            .map(|r| page_by_key.get(&r.page).copied())
            .collect();
        let rows = index
            .rows
            .iter()
            .zip(&row_page)
            .map(|(r, page)| {
                let mut words = r.keywords.clone();
                if let Some(p) = page.map(|p| &index.pages[p]) {
                    for w in [&p.key, &p.title, &p.keywords] {
                        words.push(' ');
                        words.push_str(w);
                    }
                }
                Candidate::new(r.source_title.as_deref(), r.title.as_deref(), &words)
            })
            .collect();
        let row_group = index
            .rows
            .iter()
            .map(|r| r.group.as_ref().and_then(|g| row_by_key.get(g).copied()))
            .collect();
        Self {
            revision: index.revision,
            rows,
            row_page,
            row_group,
            row_by_key,
            page_by_key,
        }
    }

    /// Strata's `find_matches` for a normalised, non-empty query.
    pub fn find(&self, query: &str) -> Matches {
        let mut m = Matches {
            rows: vec![false; self.rows.len()],
            pages: vec![false; self.page_by_key.len()],
            best: None,
        };
        if query.chars().nth(MAX_QUERY_CHARS).is_some() {
            return m;
        }
        let members = |i: &usize| self.row_group[*i].is_some();
        for i in (0..self.rows.len()).filter(|i| !members(i)) {
            if let (Some(score), Some(page)) = (self.rows[i].score(query), self.row_page[i]) {
                m.record(i, i, page, score);
            }
        }
        let found = m.rows.clone();
        for i in (0..self.rows.len()).filter(members) {
            let Some(group) = self.row_group[i] else {
                continue;
            };
            // A member on a page the index leaves out, or of a group on one, never shows.
            if self.row_page[i].is_none() || self.row_page[group].is_none() {
                continue;
            }
            let scored = self.rows[i].score(query);
            if found[group] || scored.is_some() {
                m.rows[i] = true;
            }
            // A member counts for its group: the group row shows, and its page can open.
            if let (Some(score), Some(page)) = (scored, self.row_page[group]) {
                m.record(group, i, page, score);
            }
        }
        m
    }
}

/// What one query found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Matches {
    /// By row index: shown.
    pub rows: Vec<bool>,
    /// By page index: holds a shown row.
    pub pages: Vec<bool>,
    /// The best record's score, matching row and page.
    pub best: Option<(i32, usize, usize)>,
}

impl Matches {
    fn record(&mut self, shown: usize, matched: usize, page: usize, score: i32) {
        self.rows[shown] = true;
        self.rows[matched] = true;
        self.pages[page] = true;
        if self.best.is_none_or(|(best, ..)| score > best) {
            self.best = Some((score, matched, page));
        }
    }

    pub fn none(&self) -> bool {
        !self.rows.contains(&true)
    }

    pub fn count(&self) -> usize {
        self.rows.iter().filter(|r| **r).count()
    }
}

pub fn title_ranges(title: &str, query: &str) -> Vec<Range<usize>> {
    let words = words(title);
    let joined = Word::join(&words);
    let mut out = Vec::new();
    for q in query.split_whitespace() {
        let mut found = false;
        for word in &words {
            if let Some(at) = word.find(q) {
                out.extend(word.original(at..at + q.len()));
                found = true;
            }
        }
        if !found && let Some(at) = joined.find(q) {
            out.extend(joined.original(at..at + q.len()));
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

#[derive(Default)]
struct Word {
    lower: String,
    /// For each character of `lower`: its start there, and the title character it came from.
    chars: Vec<(usize, Range<usize>)>,
}

impl Word {
    /// Where `q` matches: a prefix, or for a Han or kana query a run anywhere.
    fn find(&self, q: &str) -> Option<usize> {
        if self.lower.starts_with(q) {
            Some(0)
        } else if is_unspaced_query(q) {
            self.lower.find(q)
        } else {
            None
        }
    }

    /// The words run together, as the matcher's joined title.
    fn join(words: &[Word]) -> Word {
        let mut out = Word::default();
        for w in words {
            let base = out.lower.len();
            out.chars
                .extend(w.chars.iter().map(|(at, src)| (base + at, src.clone())));
            out.lower.push_str(&w.lower);
        }
        out
    }

    fn original(&self, range: Range<usize>) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for (_, src) in self
            .chars
            .iter()
            .filter(|(at, _)| range.start <= *at && *at < range.end)
        {
            match out.last_mut() {
                Some(last) if src.start <= last.end => last.end = last.end.max(src.end),
                _ => out.push(src.clone()),
            }
        }
        out
    }
}

fn words(title: &str) -> Vec<Word> {
    let lower = title.to_lowercase();
    let mut out = Vec::new();
    let mut current: Option<Word> = None;
    let mut from = 0;
    for (at, c) in title.char_indices() {
        let len: usize = c.to_lowercase().map(char::len_utf8).sum();
        for l in lower[from..from + len].chars() {
            if l.is_alphanumeric() {
                let w = current.get_or_insert_with(Word::default);
                w.chars.push((w.lower.len(), at..at + c.len_utf8()));
                w.lower.push(l);
            } else if let Some(w) = current.take() {
                out.push(w);
            }
        }
        from += len;
    }
    out.extend(current);
    out
}
