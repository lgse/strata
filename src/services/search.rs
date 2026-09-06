// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

const RESULT_LIMIT: usize = 100;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

/// High-volume generated trees pruned before they consume the index budget. Configuration files
/// in the parent tool directories remain searchable when hidden files are shown.
const GENERATED_TREE_GLOBS: [&str; 12] = [
    "!**/.cache",
    "!**/.cargo/registry",
    "!**/.cargo/git",
    "!**/.rustup/downloads",
    "!**/.gradle/caches",
    "!**/.gradle/wrapper/dists",
    "!**/.m2/repository",
    "!**/.npm/_cacache",
    "!**/.bun/install/cache",
    "!**/node_modules",
    "!**/target",
    "!**/.venv",
];

/// Bounds worst-case index memory on an adversarially large tree. Each retained `SearchItem`
/// stores full path strings, so cost scales with path length, not just entry count: roughly
/// 60-140 MB at this cap for typical paths, but up to ~1.5-2 GB for paths near `PATH_MAX`.
const MAX_INDEX_ENTRIES: usize = 200_000;
const MAX_INDEX_DEPTH: usize = 64;
const PRIORITY_INDEX_DEPTH: usize = 2;
const INDEX_TIME_BUDGET: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchItem {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
    search_path: String,
    search_name_start: usize,
    depth: u8,
}

impl SearchItem {
    fn search_name(&self) -> &str {
        &self.search_path[self.search_name_start..]
    }
}

pub enum SearchEvent {
    Results {
        query: String,
        items: Vec<SearchItem>,
        indexing: bool,
        /// `true` if the index does not cover the full tree; results are the best matches found
        /// so far, not necessarily complete.
        truncated: bool,
    },
}

enum SearchCommand {
    Query(String),
}

#[derive(Default)]
struct WalkProgress {
    query: String,
    normalized_query: String,
    matches: Vec<(i64, SearchItem)>,
    truncated: bool,
}

pub struct SearchHandle {
    cancelled: Arc<AtomicBool>,
    commands: Sender<SearchCommand>,
}

impl SearchHandle {
    pub fn query(&self, query: &str) {
        let _sent = self
            .commands
            .send(SearchCommand::Query(query.trim().to_owned()));
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        tracing::debug!("search index cancelled");
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// Builds and searches the index entirely off the GTK thread. The UI receives only the best
/// bounded result set, so typing remains responsive even while very large trees are being walked.
pub fn index_tree(root: PathBuf, show_hidden: bool) -> (SearchHandle, Receiver<SearchEvent>) {
    index_tree_with_budget(
        root,
        show_hidden,
        MAX_INDEX_ENTRIES,
        MAX_INDEX_DEPTH,
        INDEX_TIME_BUDGET,
    )
}

fn index_tree_with_budget(
    root: PathBuf,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let _worker = std::thread::Builder::new()
        .name("strata-search-index".into())
        .spawn(move || {
            let mut index = Vec::new();
            let mut progress = WalkProgress::default();
            let mut last_publish = Instant::now();
            let walk_start = Instant::now();
            // Walk one level past `max_depth` so a directory at the cap with real children
            // yields at least one entry beyond it, letting depth truncation be detected below.
            // `hidden` must come after `standard_filters`: that bundle enables its own
            // `hidden(true)` internally, which would otherwise override this call back on.
            let mut overrides = ignore::overrides::OverrideBuilder::new(&root);
            for generated_tree in GENERATED_TREE_GLOBS {
                if let Err(error) = overrides.add(generated_tree) {
                    tracing::warn!(
                        %error,
                        pattern = generated_tree,
                        "invalid generated-tree prune glob"
                    );
                }
            }
            let overrides = match overrides.build() {
                Ok(overrides) => overrides,
                Err(error) => {
                    tracing::warn!(%error, "generated-tree prune globs failed; walking unpruned");
                    ignore::overrides::Override::empty()
                }
            };
            let mut walker = ignore::WalkBuilder::new(&root);
            walker
                .follow_links(false)
                .standard_filters(true)
                .hidden(!show_hidden)
                .require_git(false)
                .overrides(overrides);
            let priority_depth = PRIORITY_INDEX_DEPTH.min(max_depth);
            let priority_walker = walker.max_depth(Some(priority_depth)).build();
            let remaining_walker = walker.max_depth(Some(max_depth + 1)).build();

            let entries = priority_walker
                .map(|result| (true, result))
                .chain(remaining_walker.map(|result| (false, result)));
            for (priority_pass, result) in entries {
                if worker_cancelled.load(Ordering::Relaxed) {
                    return;
                }
                let entry = match result {
                    Ok(entry) if entry.depth() == 0 => continue,
                    Ok(entry) => entry,
                    Err(_) => {
                        // An unreadable directory also omits part of the tree from the index.
                        progress.truncated = true;
                        continue;
                    }
                };
                if !priority_pass && entry.depth() <= priority_depth {
                    continue;
                }
                if entry.depth() > max_depth {
                    progress.truncated = true;
                    continue;
                }
                if index.len() >= max_entries || walk_start.elapsed() >= time_budget {
                    progress.truncated = true;
                    break;
                }
                apply_pending_queries(
                    &command_receiver,
                    &event_sender,
                    &index,
                    &mut progress,
                    true,
                );
                let is_directory = entry.file_type().is_some_and(|kind| kind.is_dir());
                let depth = entry.depth().saturating_sub(1).min(MAX_INDEX_DEPTH) as u8;
                let path = entry.into_path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let search_path = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_lowercase();
                let search_name_start = search_path
                    .rfind(std::path::MAIN_SEPARATOR)
                    .map_or(0, |position| {
                        position + std::path::MAIN_SEPARATOR.len_utf8()
                    });
                let item = SearchItem {
                    name,
                    is_directory,
                    path,
                    search_path,
                    search_name_start,
                    depth,
                };
                if let Some(score) = fuzzy_score_indexed(&item, &progress.normalized_query) {
                    insert_match(&mut progress.matches, score, &item);
                }
                index.push(item);

                if !progress.query.is_empty() && last_publish.elapsed() >= PUBLISH_INTERVAL {
                    publish(&event_sender, &progress, true);
                    last_publish = Instant::now();
                }
            }

            if progress.truncated {
                tracing::warn!(
                    entries = index.len(),
                    elapsed_ms = walk_start.elapsed().as_millis() as u64,
                    "search index truncated"
                );
            } else {
                tracing::info!(
                    entries = index.len(),
                    elapsed_ms = walk_start.elapsed().as_millis() as u64,
                    "search index built"
                );
            }
            publish(&event_sender, &progress, false);
            while !worker_cancelled.load(Ordering::Relaxed) {
                match command_receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(SearchCommand::Query(next)) => {
                        let query = command_receiver
                            .try_iter()
                            .map(|SearchCommand::Query(query)| query)
                            .last()
                            .unwrap_or(next);
                        set_query(&mut progress, query);
                        progress.matches = score_index(&index, &progress.normalized_query);
                        publish(&event_sender, &progress, false);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    (
        SearchHandle {
            cancelled,
            commands: command_sender,
        },
        event_receiver,
    )
}

fn apply_pending_queries(
    receiver: &Receiver<SearchCommand>,
    sender: &Sender<SearchEvent>,
    index: &[SearchItem],
    progress: &mut WalkProgress,
    indexing: bool,
) {
    let Some(next) = receiver
        .try_iter()
        .map(|SearchCommand::Query(query)| query)
        .last()
    else {
        return;
    };
    set_query(progress, next);
    progress.matches = score_index(index, &progress.normalized_query);
    publish(sender, progress, indexing);
}

fn set_query(progress: &mut WalkProgress, query: String) {
    progress.normalized_query = query.to_lowercase();
    progress.query = query;
}

type RankedPosition = Reverse<(i64, Reverse<usize>)>;

fn score_index(index: &[SearchItem], normalized_query: &str) -> Vec<(i64, SearchItem)> {
    let worker_count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    let best = if index.len() < 50_000 || worker_count == 1 {
        score_range(index, normalized_query, 0)
    } else {
        let chunk_size = index.len().div_ceil(worker_count);
        std::thread::scope(|scope| {
            let workers = index
                .chunks(chunk_size)
                .enumerate()
                .map(|(chunk, items)| {
                    scope.spawn(move || {
                        score_range(items, normalized_query, chunk.saturating_mul(chunk_size))
                    })
                })
                .collect::<Vec<_>>();
            let mut best = BinaryHeap::with_capacity(RESULT_LIMIT + 1);
            for worker in workers {
                let candidates = match worker.join() {
                    Ok(candidates) => candidates,
                    Err(payload) => std::panic::resume_unwind(payload),
                };
                for Reverse(candidate) in candidates {
                    retain_candidate(&mut best, candidate);
                }
            }
            best
        })
    };
    let mut ranked = best
        .into_iter()
        .map(|Reverse((score, Reverse(position)))| (score, position))
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    ranked
        .into_iter()
        .map(|(score, position)| (score, index[position].clone()))
        .collect()
}

fn score_range(
    index: &[SearchItem],
    normalized_query: &str,
    position_offset: usize,
) -> BinaryHeap<RankedPosition> {
    let mut best = BinaryHeap::with_capacity(RESULT_LIMIT + 1);
    for (position, item) in index.iter().enumerate() {
        let Some(score) = fuzzy_score_indexed(item, normalized_query) else {
            continue;
        };
        retain_candidate(
            &mut best,
            (score, Reverse(position_offset.saturating_add(position))),
        );
    }
    best
}

fn retain_candidate(best: &mut BinaryHeap<RankedPosition>, candidate: (i64, Reverse<usize>)) {
    if best.len() < RESULT_LIMIT {
        best.push(Reverse(candidate));
    } else if best.peek().is_some_and(|Reverse(worst)| candidate > *worst) {
        best.pop();
        best.push(Reverse(candidate));
    }
}

fn insert_match(matches: &mut Vec<(i64, SearchItem)>, score: i64, item: &SearchItem) {
    let position = matches
        .binary_search_by(|candidate| candidate.0.cmp(&score).reverse())
        .unwrap_or_else(|position| position);
    if position < RESULT_LIMIT {
        matches.insert(position, (score, item.clone()));
        matches.truncate(RESULT_LIMIT);
    }
}

fn publish(sender: &Sender<SearchEvent>, progress: &WalkProgress, indexing: bool) {
    let _sent = sender.send(SearchEvent::Results {
        query: progress.query.clone(),
        items: progress
            .matches
            .iter()
            .map(|(_, item)| item.clone())
            .collect(),
        indexing,
        truncated: progress.truncated,
    });
}

fn fuzzy_score_indexed(item: &SearchItem, normalized_query: &str) -> Option<i64> {
    fuzzy_score_normalized(item, normalized_query, item.depth.into())
}

fn fuzzy_score_normalized(item: &SearchItem, query: &str, depth: usize) -> Option<i64> {
    if query.is_empty() {
        return None;
    }
    let search_name = item.search_name();
    let mut score = if let Some(position) = search_name.find(query) {
        10_000 - position as i64 * 12 - search_name.len() as i64
    } else if let Some(position) = item.search_path.find(query) {
        7_000 - position as i64 * 4 - item.search_path.len() as i64
    } else {
        fuzzy_subsequence_score(&item.search_path, query)?
    };
    if search_name == query {
        score += 20_000;
    }
    if item.is_directory {
        score += 20;
    }
    score -= depth.min(MAX_INDEX_DEPTH) as i64 * 32;
    Some(score)
}

fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
    if needle.is_ascii() {
        return fuzzy_ascii_subsequence_score(haystack.as_bytes(), needle.as_bytes());
    }

    let mut chars = haystack.char_indices();
    let mut previous = None;
    let mut score = 1_000i64;
    for wanted in needle.chars() {
        let (position, _) = chars.find(|(_, candidate)| *candidate == wanted)?;
        score -= position as i64;
        if previous.is_some_and(|previous| previous + wanted.len_utf8() == position) {
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
        previous = Some(position);
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

#[cfg(test)]
mod tests;
