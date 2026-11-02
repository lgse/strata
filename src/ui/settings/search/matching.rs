// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use super::targets::{KEYBOARD_SHORTCUTS, TARGETS, Target};
use crate::ui::settings::{NAVIGATION, keyboard, page_shown, translated, translated_in};

pub(super) fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn distance(a: &str, b: &str) -> usize {
    let b = b.chars().collect::<Vec<_>>();
    let mut previous = (0..=b.len()).collect::<Vec<_>>();
    for (i, a) in a.chars().enumerate() {
        let mut next = vec![i + 1];
        for (j, b) in b.iter().enumerate() {
            next.push(
                (previous[j + 1] + 1)
                    .min(next[j] + 1)
                    .min(previous[j] + usize::from(a != *b)),
            );
        }
        previous = next;
    }
    previous[b.len()]
}

/// Kana and Han ideographs.
fn is_unspaced_script(character: char) -> bool {
    matches!(
        character,
        '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{3134f}'
    )
}

fn word_score(query: &str, word: &str) -> Option<i32> {
    if query == word {
        Some(80)
    } else if word.starts_with(query) {
        Some(60)
    } else if contains_unspaced_query(query, word) {
        Some(40)
    } else if is_near_miss(query, word) {
        Some(20)
    } else {
        None
    }
}

/// Chinese and Japanese queries have no word boundaries for a prefix match to anchor on.
fn contains_unspaced_query(query: &str, word: &str) -> bool {
    query.chars().count() >= 2 && query.chars().any(is_unspaced_script) && word.contains(query)
}

fn is_near_miss(query: &str, word: &str) -> bool {
    let allowed_edits = match query.chars().count() {
        0..=3 => return false,
        4..=5 => 1,
        _ => 2,
    };
    distance(query, word) <= allowed_edits
}

pub(super) fn score(query: &str, target: &Target) -> Option<i32> {
    let aliases = format!(
        "{} {} {}",
        target.aliases,
        localized_keywords(target),
        page_aliases(target.page)
    );
    score_text(query, &Candidate::new(target.title, &aliases))
}

fn page_aliases(page: &str) -> String {
    format!(
        "{page} {} settings {}",
        page_title(page).map(crate::i18n::tr).unwrap_or_default(),
        crate::i18n::tr("Settings"),
    )
}

fn score_text(query: &str, candidate: &Candidate) -> Option<i32> {
    let mut total = 0;
    for query_word in query.split_whitespace() {
        total += candidate.best_word_score(query_word)?;
    }
    Some(total + candidate.title_bonus(query))
}

struct Candidate {
    source_title: String,
    localized_title: String,
    aliases: String,
}

impl Candidate {
    fn new(title: &str, aliases: &str) -> Self {
        let (source_title, localized_title) = if title.is_empty() {
            (String::new(), String::new())
        } else {
            (
                normalized(&translated_in("en", title)),
                normalized(&translated(title)),
            )
        };
        Self {
            source_title,
            localized_title,
            aliases: normalized(aliases),
        }
    }

    /// Titles count in full; aliases count half.
    fn best_word_score(&self, query_word: &str) -> Option<i32> {
        // Chinese and Japanese queries run across the spaces titles put around Latin words.
        let joined_title = self.localized_title.split_whitespace().collect::<String>();
        let title_score = self
            .source_title
            .split_whitespace()
            .chain(self.localized_title.split_whitespace())
            .chain([joined_title.as_str()])
            .filter_map(|word| word_score(query_word, word))
            .max();
        let alias_score = self
            .aliases
            .split_whitespace()
            .filter_map(|word| word_score(query_word, word).map(|score| score / 2))
            .max();
        title_score.into_iter().chain(alias_score).max()
    }

    fn title_bonus(&self, query: &str) -> i32 {
        let titles = [self.source_title.as_str(), self.localized_title.as_str()];
        if self.source_title.is_empty() {
            0
        } else if titles.contains(&query) {
            1000
        } else if titles.iter().any(|title| title.contains(query)) {
            200
        } else {
            0
        }
    }
}

/// Semantic keys; a missing entry adds nothing, since the English aliases are always searched.
fn localized_keywords(target: &Target) -> String {
    let key = format!("settings_keywords.{}", target.id);
    let keywords = crate::i18n::tr(&key);
    if keywords == key {
        String::new()
    } else {
        keywords
    }
}

pub(super) fn page_title(page: &str) -> Option<&'static str> {
    NAVIGATION
        .iter()
        .find(|entry| entry.page == page)
        .map(|entry| entry.label)
}

#[derive(Default)]
pub(in crate::ui::settings) struct Matches {
    pub(super) ids: HashSet<String>,
    pub(super) pages: HashSet<&'static str>,
    pub(super) best_page: Option<&'static str>,
    best_score: Option<i32>,
}

impl Matches {
    fn record(&mut self, id: String, page: &'static str, score: i32) {
        self.ids.insert(id);
        self.pages.insert(page);
        if self.best_score.is_none_or(|best| score > best) {
            self.best_score = Some(score);
            self.best_page = Some(page);
        }
    }
}

pub(super) fn find_matches(query: &str) -> Matches {
    let mut matches = Matches::default();
    if query.chars().take(129).count() > 128 {
        return matches;
    }
    record_setting_matches(query, &mut matches);
    record_keyboard_matches(query, &mut matches);
    matches
}

fn record_setting_matches(query: &str, matches: &mut Matches) {
    let shown = TARGETS.iter().filter(|target| page_shown(target.page));
    for (target, score) in shown.filter_map(|target| Some((target, score(query, target)?))) {
        matches.record(target.id.to_owned(), target.page, score);
    }
}

fn record_keyboard_matches(query: &str, matches: &mut Matches) {
    // A query for the page itself lists every row; otherwise action names
    // score as aliases, so a setting keeps a query it ties.
    let whole_page = matches.ids.contains(KEYBOARD_SHORTCUTS);
    for target in keyboard::search_targets() {
        let aliases = format!(
            "{} {} {}",
            target.names,
            target.aliases,
            page_aliases(keyboard::PAGE)
        );
        let scored = score_text(query, &Candidate::new("", &aliases));
        if whole_page || scored.is_some() {
            matches.ids.insert(target.id);
        }
        if let Some(score) = scored {
            // Keeps the table switch shown, since a row may match only in the hidden table.
            matches.record(KEYBOARD_SHORTCUTS.to_owned(), keyboard::PAGE, score);
        }
    }
}
