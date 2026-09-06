// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

const RESULT_LIMIT: usize = 100;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

/// Bounds worst-case index memory on an adversarially large tree. Each retained `SearchItem`
/// stores full path strings, so cost scales with path length, not just entry count: roughly
/// 60-140 MB at this cap for typical paths, but up to ~1.5-2 GB for paths near `PATH_MAX`.
const MAX_INDEX_ENTRIES: usize = 200_000;
const MAX_INDEX_DEPTH: usize = 64;
const INDEX_TIME_BUDGET: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchItem {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
    search_name: String,
    search_path: String,
    depth: usize,
}

impl SearchItem {
    fn new(path: PathBuf, root: &Path, is_directory: bool) -> Self {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let depth = relative.components().count().saturating_sub(1);
        let search_path = relative.to_string_lossy().to_lowercase();
        Self {
            search_name: name.to_lowercase(),
            name,
            path,
            is_directory,
            search_path,
            depth,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchCoverage {
    pub entry_limit: bool,
    pub depth_limit: bool,
    pub time_limit: bool,
    pub unreadable: bool,
}

impl SearchCoverage {
    pub fn is_partial(self) -> bool {
        self != Self::default()
    }

    pub fn message(self) -> String {
        let mut reasons = Vec::new();
        if self.entry_limit {
            reasons.push("entry limit reached");
        }
        if self.depth_limit {
            reasons.push("depth limit reached");
        }
        if self.time_limit {
            reasons.push("indexing time limit reached");
        }
        if self.unreadable {
            reasons.push("some folders could not be read");
        }
        if reasons.is_empty() {
            String::new()
        } else {
            format!("Partial search — {}", reasons.join("; "))
        }
    }
}

pub enum SearchEvent {
    Results {
        query: String,
        items: Vec<SearchItem>,
        indexing: bool,
        coverage: SearchCoverage,
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
    coverage: SearchCoverage,
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
    index_trees(vec![root], show_hidden)
}

pub fn index_trees(
    roots: Vec<PathBuf>,
    show_hidden: bool,
) -> (SearchHandle, Receiver<SearchEvent>) {
    index_trees_with_budget(
        roots,
        show_hidden,
        MAX_INDEX_ENTRIES,
        MAX_INDEX_DEPTH,
        INDEX_TIME_BUDGET,
    )
}

fn index_trees_with_budget(
    roots: Vec<PathBuf>,
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
            let mut seen = HashSet::new();
            let roots: Vec<_> = roots
                .into_iter()
                .filter(|root| seen.insert(root.clone()))
                .collect();
            let roots_set = Arc::new(seen);
            let mut walkers: Vec<_> = roots
                .into_iter()
                .map(|root| {
                    let boundaries = roots_set.clone();
                    let walker = ignore::WalkBuilder::new(&root)
                        .follow_links(false)
                        .standard_filters(true)
                        .hidden(!show_hidden)
                        .require_git(false)
                        .max_depth(Some(max_depth + 1))
                        // Nested mounts are walked separately, never through both roots.
                        .filter_entry(move |entry| {
                            entry.depth() == 0 || !boundaries.contains(entry.path())
                        })
                        .build()
                        .fuse();
                    (root, walker)
                })
                .collect();

            // Interleave roots under one shared budget so Home cannot consume the
            // entire index before a mounted drive gets its first turn.
            let mut walking = true;
            'walk: while walking {
                walking = false;
                for (root, walker) in &mut walkers {
                    if worker_cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    if walk_start.elapsed() >= time_budget {
                        progress.coverage.time_limit = true;
                        break 'walk;
                    }
                    let Some(result) = walker.next() else {
                        continue;
                    };
                    walking = true;
                    let entry = match result {
                        Ok(entry) => entry,
                        Err(_) => {
                            progress.coverage.unreadable = true;
                            continue;
                        }
                    };
                    if entry.error().is_some() {
                        progress.coverage.unreadable = true;
                    }
                    if entry.depth() == 0 {
                        continue;
                    }
                    if entry.depth() > max_depth {
                        progress.coverage.depth_limit = true;
                        continue;
                    }
                    if index.len() >= max_entries {
                        progress.coverage.entry_limit = true;
                        break 'walk;
                    }
                    apply_pending_queries(
                        &command_receiver,
                        &event_sender,
                        &index,
                        &mut progress,
                        true,
                    );
                    let is_directory = entry.file_type().is_some_and(|kind| kind.is_dir());
                    let item = SearchItem::new(entry.into_path(), root, is_directory);
                    if let Some(score) = fuzzy_score_normalized(&item, &progress.normalized_query) {
                        insert_match(&mut progress.matches, score, &item);
                    }
                    index.push(item);

                    if !progress.query.is_empty() && last_publish.elapsed() >= PUBLISH_INTERVAL {
                        publish(&event_sender, &progress, true);
                        last_publish = Instant::now();
                    }
                }
            }

            if progress.coverage.is_partial() {
                tracing::warn!(
                    entries = index.len(),
                    elapsed_ms = walk_start.elapsed().as_millis() as u64,
                    coverage = ?progress.coverage,
                    "search index partial"
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
                        progress.query = command_receiver
                            .try_iter()
                            .map(|SearchCommand::Query(query)| query)
                            .last()
                            .unwrap_or(next);
                        progress.normalized_query = progress.query.to_lowercase();
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
    progress.query = next;
    progress.normalized_query = progress.query.to_lowercase();
    progress.matches = score_index(index, &progress.normalized_query);
    publish(sender, progress, indexing);
}

fn score_index(index: &[SearchItem], normalized_query: &str) -> Vec<(i64, SearchItem)> {
    let mut matches = Vec::with_capacity(RESULT_LIMIT);
    for item in index {
        if let Some(score) = fuzzy_score_normalized(item, normalized_query) {
            insert_match(&mut matches, score, item);
        }
    }
    matches
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
        coverage: progress.coverage,
    });
}

/// Scores ordered character matches, strongly preferring names, contiguous runs and word/path
/// boundaries. Exact substrings rank ahead of looser fuzzy matches.
fn fuzzy_score_normalized(item: &SearchItem, query: &str) -> Option<i64> {
    if query.is_empty() {
        return None;
    }
    let mut score = if let Some(position) = item.search_name.find(query) {
        10_000 - position as i64 * 12 - item.search_name.len() as i64
    } else if let Some(position) = item.search_path.find(query) {
        7_000 - position as i64 * 4 - item.search_path.len() as i64
    } else {
        fuzzy_subsequence_score(&item.search_path, query)?
    };
    if item.search_name == query {
        score += 20_000;
    }
    if item.is_directory {
        score += 20;
    }
    // Prefer nearby matches without allowing proximity to outweigh match quality.
    score -= item.depth.min(MAX_INDEX_DEPTH) as i64 * 32;
    Some(score)
}

fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
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

#[cfg(test)]
mod tests;
