// SPDX-License-Identifier: MIT

/// Inputs must be Unicode-folded.
pub(crate) fn filter_name_matches(name: &str, query: &str) -> bool {
    let Some((prefix, rest)) = query.split_once('*') else {
        return name.contains(query)
            || typo_matches_word(name, query)
            || (query.contains(char::is_whitespace)
                && text_matches_words(name, query.split_whitespace()));
    };
    let Some(name) = name.strip_prefix(prefix) else {
        return false;
    };
    let (middle, suffix) = rest.rsplit_once('*').unwrap_or(("", rest));
    let Some(mut remaining) = name.strip_suffix(suffix) else {
        return false;
    };
    // Reserve the anchored suffix first so interior matches cannot consume it.
    // Literal segments advance through disjoint slices without wildcard backtracking.
    for segment in middle.split('*').filter(|segment| !segment.is_empty()) {
        let Some(position) = remaining.find(segment) else {
            return false;
        };
        remaining = &remaining[position + segment.len()..];
    }
    true
}

pub(crate) fn filter_query_allows_typos(query: &str) -> bool {
    query.chars().take(4).count() == 4 && query.chars().all(char::is_alphanumeric)
}

fn text_matches_word(text: &str, word: &str) -> bool {
    text.contains(word) || typo_matches_word(text, word)
}

pub(crate) fn text_matches_words<'a>(text: &str, mut words: impl Iterator<Item = &'a str>) -> bool {
    let mut empty = true;
    let matches = words.all(|word| {
        empty = false;
        text_matches_word(text, word)
    });
    !empty && matches
}

fn typo_matches_word(name: &str, query: &str) -> bool {
    if !filter_query_allows_typos(query) {
        return false;
    }
    name.split(|character: char| !character.is_alphanumeric())
        .any(|word| within_one_edit(word, query))
}

fn within_one_edit(word: &str, query: &str) -> bool {
    let mut word = word.chars();
    let mut query = query.chars();
    // Character-aligned suffixes can be compared without decoding them again.
    loop {
        let word_tail = word.as_str();
        let query_tail = query.as_str();
        match (word.next(), query.next()) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => {
                return word.as_str() == query.as_str()
                    || word_tail == query.as_str()
                    || word.as_str() == query_tail
                    || (word.next() == Some(right)
                        && query.next() == Some(left)
                        && word.as_str() == query.as_str());
            }
            (None, None) => return true,
            (Some(_), None) => return word.next().is_none(),
            (None, Some(_)) => return query.next().is_none(),
        }
    }
}

#[cfg(test)]
mod tests;
