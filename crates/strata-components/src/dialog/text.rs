// SPDX-License-Identifier: MIT

use std::ops::Range;

use super::KOREAN_WORD_MAX;

const WORD_JOINER: char = '\u{2060}';

/// The longest trailing particle, such as "을(를)" or "에서:", kept with the word before it.
const MAX_PARTICLE_CHARS: usize = 6;

fn is_hangul_syllable(c: char) -> bool {
    ('\u{ac00}'..='\u{d7a3}').contains(&c)
}

pub fn keep_words_whole(text: &str) -> String {
    let ranges = korean_word_ranges(text);
    if ranges.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 3 * text.chars().count());
    let mut ranges = ranges.into_iter().peekable();
    let mut prev_in: Option<usize> = None;
    for (i, c) in text.char_indices() {
        while ranges.peek().is_some_and(|r| r.end <= i) {
            ranges.next();
        }
        let inside = ranges.peek().is_some_and(|r| r.contains(&i));
        let range_start = ranges.peek().map(|r| r.start);
        if inside && prev_in.is_some() && prev_in == range_start {
            out.push(WORD_JOINER);
        }
        prev_in = if inside { range_start } else { None };
        out.push(c);
    }
    out
}

pub(crate) fn korean_word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        if !c.is_whitespace() {
            start.get_or_insert(index);
            continue;
        }
        let Some(word_start) = start.take() else {
            continue;
        };
        let word = &text[word_start..index];
        let length = word.chars().count();
        if length > 1 && length <= KOREAN_WORD_MAX && word.chars().any(is_hangul_syllable) {
            ranges.push(word_start..index);
        } else if let Some(attached) = attached_particle_start(word) {
            ranges.push(word_start + attached..index);
        }
    }
    ranges
}

fn attached_particle_start(word: &str) -> Option<usize> {
    let is_particle_char =
        |c: char| is_hangul_syllable(c) || matches!(c, '(' | ')' | ':' | ',' | '.');
    let mut suffix_start = word.len();
    let mut suffix_chars = 0;
    for (index, c) in word.char_indices().rev() {
        if !is_particle_char(c) {
            break;
        }
        suffix_start = index;
        suffix_chars += 1;
    }
    if suffix_chars > MAX_PARTICLE_CHARS {
        return parenthesised_particle_start(word);
    }
    if !word[suffix_start..].chars().any(is_hangul_syllable) {
        return None;
    }
    previous_char_start(word, suffix_start)
}

fn previous_char_start(word: &str, index: usize) -> Option<usize> {
    word[..index].char_indices().next_back().map(|(i, _)| i)
}

fn parenthesised_particle_start(word: &str) -> Option<usize> {
    let is_short_hangul = |text: &str, min: usize| {
        (min..=2).contains(&text.chars().count()) && text.chars().all(is_hangul_syllable)
    };
    let trimmed = word.trim_end_matches([':', ',', '.']);
    let close = trimmed.rfind(')')?;
    let open = trimmed[..close].rfind('(')?;
    let after = &trimmed[close + 1..];
    if !is_short_hangul(&trimmed[open + 1..close], 1) || !is_short_hangul(after, 0) {
        return None;
    }
    let particle = if after.is_empty() {
        previous_char_start(word, open)
            .filter(|&i| word[i..open].chars().all(is_hangul_syllable))?
    } else {
        open
    };
    previous_char_start(word, particle)
}
