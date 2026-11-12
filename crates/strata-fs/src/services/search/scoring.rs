// SPDX-License-Identifier: MIT
//! Ranking index items against a query, sequentially or across worker threads.

use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
use super::RESULT_LIMIT;
use super::{
    SearchItem,
    path_match::{self, Frecency, PathMatcher, PathQuery},
    pattern::{self, filter_name_matches},
};

pub(super) type NameScorer = fn(&SearchItem, &str) -> Option<i64>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefusedFolders {
    pub trees: Vec<PathBuf>,
    pub folder: Option<PathBuf>,
}

impl RefusedFolders {
    pub fn refuses(&self, path: &Path) -> bool {
        self.folder.as_deref() == Some(path) || self.trees.iter().any(|tree| path.starts_with(tree))
    }
}

#[derive(Clone)]
pub(super) enum SearchScorer {
    Name(NameScorer),
    Paths {
        frecency: Arc<Frecency>,
        /// `Some` restricts hits to directories and applies transfer exclusions.
        folders: Option<Arc<RefusedFolders>>,
    },
}

impl SearchScorer {
    pub(super) fn prepare<'a>(&'a self, normalized_query: &'a str) -> QueryScorer<'a> {
        match self {
            Self::Name(scorer) => QueryScorer::Name(*scorer, normalized_query),
            Self::Paths {
                frecency,
                folders: Some(refused),
            } if normalized_query.trim().is_empty() => QueryScorer::Folders(frecency, refused),
            Self::Paths { frecency, folders } => QueryScorer::Paths(PathScorer {
                matcher: PathMatcher::new(&PathQuery::parse(normalized_query)),
                query: normalized_query,
                frecency,
                folders: folders.as_deref(),
            }),
        }
    }

    pub(super) fn lists_without_query(&self) -> bool {
        matches!(
            self,
            Self::Paths {
                folders: Some(_),
                ..
            }
        )
    }
}

pub(super) struct PathScorer<'a> {
    matcher: PathMatcher,
    query: &'a str,
    frecency: &'a Frecency,
    folders: Option<&'a RefusedFolders>,
}

impl PathScorer<'_> {
    fn refuses(&self, item: &SearchItem) -> bool {
        self.folders
            .is_some_and(|refused| !item.is_directory || refused.refuses(&item.path))
    }

    fn score(&mut self, item: &SearchItem) -> Option<i64> {
        if self.refuses(item) {
            return None;
        }
        let text = item.path_score(&mut self.matcher)?;
        let exact = if self.folders.is_some() {
            path_match::exact_bonus(&item.search_path, item.search_name_start, self.query)
        } else {
            0
        };
        Some(path_match::rank(text, self.frecency.bias(&item.path, item.is_directory)) + exact)
    }
}

pub(super) enum QueryScorer<'a> {
    Name(NameScorer, &'a str),
    Paths(PathScorer<'a>),
    Folders(&'a Frecency, &'a RefusedFolders),
}

impl QueryScorer<'_> {
    pub(super) fn score(&mut self, item: &SearchItem) -> Option<i64> {
        match self {
            Self::Name(scorer, query) => scorer(item, query),
            Self::Folders(frecency, refused) => (item.is_directory && !refused.refuses(&item.path))
                .then(|| {
                    frecency.bias(&item.path, true) * (i64::from(u8::MAX) + 1)
                        - i64::from(item.depth)
                }),
            Self::Paths(scorer) => scorer.score(item),
        }
    }
}

/// One scored index position. Higher ranks first; equal scores fall back to the folded
/// name and then the path, so the walk's enumeration order never decides.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Ranked<'a> {
    score: i64,
    tie: Reverse<(&'a str, &'a str, usize)>,
}

impl<'a> Ranked<'a> {
    fn new(score: i64, item: &'a SearchItem, position: usize) -> Self {
        Self {
            score,
            tie: Reverse((item.search_name(), &item.search_path, position)),
        }
    }

    fn position(&self) -> usize {
        self.tie.0.2
    }
}

type RankedPosition<'a> = Reverse<Ranked<'a>>;

#[cfg(test)]
pub(super) fn score_index(
    index: &[SearchItem],
    query: &str,
    scorer: SearchScorer,
) -> Vec<(i64, SearchItem)> {
    score_index_with_limit(index, query, &scorer, RESULT_LIMIT)
}

/// What one ranking pass keeps: the best `limit` items for the normalized query.
#[derive(Clone, Copy)]
struct Ranking<'a> {
    normalized_query: &'a str,
    scorer: &'a SearchScorer,
    limit: usize,
}

fn score_in_parallel<'a>(
    index: &'a [SearchItem],
    ranking: Ranking<'_>,
    worker_count: usize,
) -> BinaryHeap<RankedPosition<'a>> {
    let chunk_size = index.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let workers = index
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk, items)| {
                scope.spawn(move || score_range(items, chunk * chunk_size, ranking))
            })
            .collect::<Vec<_>>();
        let mut best = BinaryHeap::with_capacity(ranking.limit + 1);
        for worker in workers {
            let candidates = match worker.join() {
                Ok(candidates) => candidates,
                Err(payload) => std::panic::resume_unwind(payload),
            };
            for Reverse(candidate) in candidates {
                retain_candidate(&mut best, candidate, ranking.limit);
            }
        }
        best
    })
}

pub(super) fn score_index_with_limit(
    index: &[SearchItem],
    normalized_query: &str,
    scorer: &SearchScorer,
    limit: usize,
) -> Vec<(i64, SearchItem)> {
    let ranking = Ranking {
        normalized_query,
        scorer,
        limit,
    };
    let worker_count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    let best = if index.len() < 50_000 || worker_count == 1 {
        score_range(index, 0, ranking)
    } else {
        score_in_parallel(index, ranking, worker_count)
    };
    best.into_sorted_vec()
        .into_iter()
        .map(|Reverse(ranked)| (ranked.score, index[ranked.position()].clone()))
        .collect()
}

fn score_range<'a>(
    index: &'a [SearchItem],
    position_offset: usize,
    ranking: Ranking<'_>,
) -> BinaryHeap<RankedPosition<'a>> {
    let mut best = BinaryHeap::with_capacity(ranking.limit + 1);
    let mut scorer = ranking.scorer.prepare(ranking.normalized_query);
    for (position, item) in index.iter().enumerate() {
        let Some(score) = scorer.score(item) else {
            continue;
        };
        retain_candidate(
            &mut best,
            Ranked::new(score, item, position_offset + position),
            ranking.limit,
        );
    }
    best
}

fn retain_candidate<'a>(
    best: &mut BinaryHeap<RankedPosition<'a>>,
    candidate: Ranked<'a>,
    limit: usize,
) {
    if best.len() < limit {
        best.push(Reverse(candidate));
    } else if best.peek().is_some_and(|Reverse(worst)| candidate > *worst) {
        best.pop();
        best.push(Reverse(candidate));
    }
}

#[cfg(test)]
pub(super) fn insert_match(matches: &mut Vec<(i64, SearchItem)>, score: i64, item: &SearchItem) {
    insert_match_with_limit(matches, score, item, RESULT_LIMIT);
}

pub(super) fn insert_match_with_limit(
    matches: &mut Vec<(i64, SearchItem)>,
    score: i64,
    item: &SearchItem,
    limit: usize,
) {
    let ranked = Ranked::new(score, item, 0);
    let position =
        matches.partition_point(|(kept_score, kept)| Ranked::new(*kept_score, kept, 0) > ranked);
    if position < limit {
        matches.insert(position, (score, item.clone()));
        matches.truncate(limit);
    }
}

pub(super) fn filter_score_normalized(item: &SearchItem, query: &str) -> Option<i64> {
    if !filter_name_matches(item.search_name(), query) {
        return None;
    }
    if !query.contains('*') && item.search_name().contains(query) {
        return fuzzy_score_normalized(item, query);
    }
    Some(i64::from(item.is_directory) * 20 - i64::from(item.depth) * 32)
}

/// The tier and base score of a query that is not a contiguous substring.
fn scattered_match(item: &SearchItem, query: &str) -> Option<(i64, i64)> {
    if !query.contains(char::is_whitespace) {
        return Some((1, fuzzy_subsequence_score(&item.search_path, query)?));
    }
    let search_name = item.search_name();
    if pattern::text_matches_words(search_name, query.split_whitespace()) {
        Some((3, 5_000 - search_name.len() as i64))
    } else if pattern::text_matches_words(&item.search_path, query.split_whitespace()) {
        Some((2, 3_000 - item.search_path.len() as i64))
    } else {
        Some((1, fuzzy_subsequence_score(&item.search_path, query)?))
    }
}

fn match_tier(item: &SearchItem, query: &str) -> Option<(i64, i64)> {
    let search_name = item.search_name();
    if let Some(position) = search_name.find(query) {
        Some((5, 10_000 - position as i64 * 12 - search_name.len() as i64))
    } else if let Some(position) = item.search_path.find(query) {
        Some((
            4,
            7_000 - position as i64 * 4 - item.search_path.len() as i64,
        ))
    } else {
        scattered_match(item, query)
    }
}

pub(super) fn fuzzy_score_normalized(item: &SearchItem, query: &str) -> Option<i64> {
    if query.trim().is_empty() {
        return None;
    }
    let (tier, mut score) = match_tier(item, query)?;
    if item.search_name() == query {
        score += 20_000;
    }
    if item.is_directory {
        score += 20;
    }
    score -= i64::from(item.depth) * 32;
    // Keep penalties and fuzzy bonuses inside their match tier.
    Some((tier << 32) + score.clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
}

pub(super) fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
    if needle.is_ascii() {
        return fuzzy_ascii_subsequence_score(haystack.as_bytes(), needle.as_bytes());
    }
    let mut chars = haystack.char_indices();
    let mut previous_end = None;
    let mut score = 1_000i64;
    for wanted in needle.chars() {
        let (position, _) = chars.find(|(_, candidate)| *candidate == wanted)?;
        score -= position as i64;
        if previous_end == Some(position) {
            score += 80;
        }
        if position == 0
            || haystack[..position]
                .chars()
                .next_back()
                .is_some_and(|character| matches!(character, '/' | '-' | '_' | ' ' | '.'))
        {
            score += 45;
        }
        previous_end = Some(position + wanted.len_utf8());
    }
    Some(score)
}

fn fuzzy_ascii_subsequence_score(haystack: &[u8], needle: &[u8]) -> Option<i64> {
    let mut offset = 0;
    let mut previous = None;
    let mut score = 1_000i64;
    for wanted in needle {
        let relative = haystack[offset..]
            .iter()
            .position(|candidate| candidate == wanted)?;
        let position = offset + relative;
        score -= position as i64;
        if previous.is_some_and(|previous| previous + 1 == position) {
            score += 80;
        }
        if position == 0
            || haystack
                .get(position - 1)
                .is_some_and(|character| matches!(character, b'/' | b'-' | b'_' | b' ' | b'.'))
        {
            score += 45;
        }
        previous = Some(position);
        offset = position + 1;
    }
    Some(score)
}
