// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
};

use frizbee::{CaseMatching, Config, Matcher, Pattern};

use super::search::fold_for_search;

// Name matches must outrank path-only matches regardless of frecency.
const NAME_TERM_TIER: i64 = 1 << 32;
// About four matched characters: frecency should only reorder similar matches.
const MAX_FRECENCY_BIAS: f64 = 48.0;
const FRECENCY_BIAS_SCALE: f64 = 12.0;
const EXACT_NAME_BONUS: i64 = 4 * MAX_FRECENCY_BIAS as i64;
const EXACT_PATH_BONUS: i64 = NAME_TERM_TIER / 2;
const ANCESTOR_DECAY: f64 = 0.5;

#[derive(Clone, Debug)]
pub(crate) struct PathQuery {
    terms: Vec<Pattern>,
    exclusions: Vec<Pattern>,
}

impl PathQuery {
    pub(crate) fn parse(text: &str) -> Self {
        let (exclusions, terms): (Vec<_>, Vec<_>) = Pattern::parse_query(&fold_for_search(text))
            .into_iter()
            .partition(|pattern| pattern.negated);
        Self {
            terms,
            exclusions: exclusions
                .into_iter()
                .map(|pattern| pattern.negated(false))
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TextScore {
    name_terms: u32,
    quality: u32,
}

pub(crate) fn rank(text: TextScore, frecency_bias: i64) -> i64 {
    i64::from(text.name_terms) * NAME_TERM_TIER + i64::from(text.quality) + frecency_bias
}

pub(crate) fn exact_bonus(path: &str, name_start: usize, query: &str) -> i64 {
    if path == query {
        EXACT_PATH_BONUS
    } else if &path[name_start..] == query {
        EXACT_NAME_BONUS
    } else {
        0
    }
}

/// Reuse across candidates to retain matcher scratch space.
#[derive(Clone, Debug)]
pub(crate) struct PathMatcher {
    terms: Vec<Matcher>,
    exclusions: Vec<Matcher>,
}

impl PathMatcher {
    pub(crate) fn new(query: &PathQuery) -> Self {
        // Queries and haystacks are both folded, so case never needs ignoring.
        let config = Config::default().casing(CaseMatching::Respect);
        let compile = |patterns: &[Pattern]| {
            patterns
                .iter()
                .map(|pattern| Matcher::new(pattern.clone(), &config))
                .collect()
        };
        Self {
            terms: compile(&query.terms),
            exclusions: compile(&query.exclusions),
        }
    }

    /// `path` must be folded; `name_start` is a byte boundary.
    pub(crate) fn score(&mut self, path: &str, name_start: usize) -> Option<TextScore> {
        if self.terms.is_empty() && self.exclusions.is_empty() {
            return None;
        }
        if self
            .exclusions
            .iter_mut()
            .any(|matcher| matcher.match_one(path, 0).is_some())
        {
            return None;
        }
        let name = &path[name_start..];
        let mut score = TextScore::default();
        for matcher in &mut self.terms {
            if let Some(found) = matcher.match_one(name, 0) {
                score.name_terms += 1;
                score.quality += u32::from(found.score);
            } else {
                score.quality += u32::from(matcher.match_one(path, 0)?.score);
            }
        }
        Some(score)
    }

    /// Returns non-overlapping byte ranges in the original, unfolded text.
    pub(crate) fn highlight(&mut self, path: &str, name_start: usize) -> Vec<Range<usize>> {
        let (folded, spans) = fold_with_spans(path);
        let folded_name_start = spans.partition_point(|span| span.start < name_start);
        let name = &folded[folded_name_start..];
        let mut matched = Vec::new();
        for matcher in &mut self.terms {
            if let Some(found) = matcher.match_one_indices(name, 0) {
                matched.extend(
                    found
                        .indices
                        .iter()
                        .map(|&index| folded_name_start + index as usize),
                );
            } else if let Some(found) = matcher.match_one_indices(&folded, 0) {
                matched.extend(found.indices.iter().map(|&index| index as usize));
            }
        }
        let mut ranges: Vec<Range<usize>> = matched
            .into_iter()
            .filter_map(|index| spans.get(index).cloned())
            .collect();
        ranges.sort_unstable_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }

    pub(crate) fn name_highlight(&mut self, root: &Path, path: &Path) -> Vec<Range<usize>> {
        let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
        let name_len = path
            .file_name()
            .map_or(0, |name| name.to_string_lossy().len());
        let name_start = relative.len().saturating_sub(name_len);
        self.highlight(&relative, name_start)
            .into_iter()
            .filter(|range| range.end > name_start)
            .map(|range| range.start.max(name_start) - name_start..range.end - name_start)
            .collect()
    }
}

// Unlike scoring, this preserves source spans without NFC composition.
fn fold_with_spans(text: &str) -> (String, Vec<Range<usize>>) {
    let mut folded = String::with_capacity(text.len());
    let mut spans = Vec::with_capacity(text.len());
    for (start, character) in text.char_indices() {
        let span = start..start + character.len_utf8();
        for lower in character.to_lowercase() {
            folded.push(lower);
            spans.extend(std::iter::repeat_n(span.clone(), lower.len_utf8()));
        }
    }
    (folded, spans)
}

/// Snapshot for search workers, which cannot access the UI-owned history.
#[derive(Debug, Default)]
pub(crate) struct Frecency {
    root_len: usize,
    folders: HashMap<PathBuf, f64>,
}

impl Frecency {
    // The root and its ancestors would bias every result equally.
    pub(crate) fn within(root: &Path, folders: impl IntoIterator<Item = (PathBuf, f64)>) -> Self {
        Self {
            root_len: root.as_os_str().len(),
            folders: folders
                .into_iter()
                .filter(|(folder, frecency)| {
                    *frecency > 0.0 && folder != root && folder.starts_with(root)
                })
                .collect(),
        }
    }

    // Prefer the nearest visited ancestor, not the most frequently visited one.
    pub(crate) fn bias(&self, path: &Path, is_directory: bool) -> i64 {
        if self.folders.is_empty() {
            return 0;
        }
        let nearest = if is_directory {
            Some(path)
        } else {
            path.parent()
        };
        nearest
            .into_iter()
            .flat_map(Path::ancestors)
            .take_while(|folder| folder.as_os_str().len() > self.root_len)
            .enumerate()
            .find_map(|(distance, folder)| {
                self.folders
                    .get(folder)
                    .map(|frecency| frecency * ANCESTOR_DECAY.powi(distance as i32))
            })
            .map_or(0, |frecency| {
                (frecency.ln_1p() * FRECENCY_BIAS_SCALE).min(MAX_FRECENCY_BIAS) as i64
            })
    }
}

#[cfg(test)]
mod tests;
