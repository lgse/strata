// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::Cell,
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, RwLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    IndexChanged,
}

#[derive(Default)]
struct WalkProgress {
    query: String,
    normalized_query: String,
    matches: Vec<(i64, SearchItem)>,
}

struct IndexLifecycle {
    active_sessions: usize,
    retired: bool,
}

struct SharedIndex {
    items: RwLock<Vec<SearchItem>>,
    subscribers: Mutex<Vec<(usize, Sender<SearchCommand>)>>,
    next_subscriber: AtomicUsize,
    lifecycle: Mutex<IndexLifecycle>,
    indexing: AtomicBool,
    truncated: AtomicBool,
}

impl SharedIndex {
    fn new() -> Self {
        Self {
            items: RwLock::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
            next_subscriber: AtomicUsize::new(1),
            lifecycle: Mutex::new(IndexLifecycle {
                active_sessions: 1,
                retired: false,
            }),
            indexing: AtomicBool::new(true),
            truncated: AtomicBool::new(false),
        }
    }

    fn subscribe(&self, sender: Sender<SearchCommand>) -> usize {
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((id, sender));
        id
    }

    fn unsubscribe(&self, id: usize) {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(subscriber_id, _)| *subscriber_id != id);
    }

    fn broadcast_change(&self) {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(_, subscriber)| subscriber.send(SearchCommand::IndexChanged).is_ok());
    }

    fn try_acquire(&self) -> bool {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lifecycle.retired {
            return false;
        }
        lifecycle.active_sessions += 1;
        true
    }

    fn release(&self) {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lifecycle.active_sessions = lifecycle.active_sessions.saturating_sub(1);
        if lifecycle.active_sessions == 0 {
            lifecycle.retired = true;
        }
    }

    fn is_retired(&self) -> bool {
        self.lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retired
    }
}

type IndexRegistry = HashMap<(PathBuf, bool), Weak<SharedIndex>>;
static SHARED_INDEXES: OnceLock<Mutex<IndexRegistry>> = OnceLock::new();

pub struct SearchHandle {
    cancelled: Arc<AtomicBool>,
    commands: Sender<SearchCommand>,
    index: Arc<SharedIndex>,
    subscriber_id: usize,
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
        tracing::debug!("search session cancelled");
        self.cancelled.store(true, Ordering::Release);
        self.index.unsubscribe(self.subscriber_id);
        self.index.release();
    }
}

/// Builds and searches the index entirely off the GTK thread. Searches with the same root and
/// hidden-file policy share indexed paths while keeping independent queries and result streams.
pub fn index_tree(root: PathBuf, show_hidden: bool) -> (SearchHandle, Receiver<SearchEvent>) {
    let key = (root.clone(), show_hidden);
    let registry = SHARED_INDEXES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|_, index| index.strong_count() > 0);
    let shared = registry
        .get(&key)
        .and_then(Weak::upgrade)
        .filter(|index| index.try_acquire());
    let index = if let Some(index) = shared {
        index
    } else {
        let index = Arc::new(SharedIndex::new());
        registry.insert(key, Arc::downgrade(&index));
        start_indexer(
            index.clone(),
            root,
            show_hidden,
            MAX_INDEX_ENTRIES,
            MAX_INDEX_DEPTH,
            INDEX_TIME_BUDGET,
        );
        index
    };
    drop(registry);
    start_search_session(index)
}

#[cfg(test)]
fn index_tree_with_budget(
    root: PathBuf,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let index = Arc::new(SharedIndex::new());
    start_indexer(
        index.clone(),
        root,
        show_hidden,
        max_entries,
        max_depth,
        time_budget,
    );
    start_search_session(index)
}

fn start_search_session(index: Arc<SharedIndex>) -> (SearchHandle, Receiver<SearchEvent>) {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let subscriber_id = index.subscribe(command_sender.clone());
    let worker_cancelled = cancelled.clone();
    let worker_index = index.clone();
    let _worker = std::thread::Builder::new()
        .name("strata-search-query".into())
        .spawn(move || {
            run_search_session(
                &worker_index,
                &worker_cancelled,
                &command_receiver,
                &event_sender,
            );
        });
    let _initial = command_sender.send(SearchCommand::IndexChanged);
    (
        SearchHandle {
            cancelled,
            commands: command_sender,
            index,
            subscriber_id,
        },
        event_receiver,
    )
}

fn run_search_session(
    index: &SharedIndex,
    cancelled: &AtomicBool,
    commands: &Receiver<SearchCommand>,
    events: &Sender<SearchEvent>,
) {
    let mut progress = WalkProgress::default();
    let mut indexed_items = 0;
    while !cancelled.load(Ordering::Acquire) {
        let first = match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut next_query = None;
        let mut index_changed = false;
        for command in std::iter::once(first).chain(commands.try_iter()) {
            match command {
                SearchCommand::Query(query) => next_query = Some(query),
                SearchCommand::IndexChanged => index_changed = true,
            }
        }
        let query_changed = next_query.is_some();
        let items = index
            .items
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(query) = next_query {
            set_query(&mut progress, query);
            progress.matches = if progress.normalized_query.is_empty() {
                Vec::new()
            } else {
                score_index(&items, &progress.normalized_query)
            };
        } else if index_changed && !progress.normalized_query.is_empty() {
            for item in &items[indexed_items.min(items.len())..] {
                if let Some(score) = fuzzy_score_indexed(item, &progress.normalized_query) {
                    insert_match(&mut progress.matches, score, item);
                }
            }
        }
        indexed_items = items.len();
        drop(items);

        let indexing = index.indexing.load(Ordering::Acquire);
        if query_changed || (index_changed && (!progress.query.is_empty() || !indexing)) {
            publish(
                events,
                &progress,
                indexing,
                index.truncated.load(Ordering::Acquire),
            );
        }
    }
}

fn start_indexer(
    index: Arc<SharedIndex>,
    root: PathBuf,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) {
    let worker_index = index.clone();
    let worker = std::thread::Builder::new()
        .name("strata-search-index".into())
        .spawn(move || {
            build_index(
                &worker_index,
                root,
                show_hidden,
                max_entries,
                max_depth,
                time_budget,
            );
        });
    if let Err(error) = worker {
        tracing::error!(%error, "search index worker failed to start");
        index.truncated.store(true, Ordering::Release);
        index.indexing.store(false, Ordering::Release);
        index.broadcast_change();
    }
}

fn build_index(
    index: &SharedIndex,
    root: PathBuf,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) {
    let mut indexed_entries = 0;
    let mut pending_items = Vec::with_capacity(256);
    let mut truncated = false;
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

    let has_deeper_entries = Cell::new(false);
    let priority_entries = priority_walker.map(|result| {
        if result.as_ref().is_ok_and(|entry| {
            entry.depth() == priority_depth
                && entry
                    .file_type()
                    .is_some_and(|file_type| file_type.is_dir())
        }) {
            has_deeper_entries.set(true);
        }
        (true, result)
    });
    let remaining_entries = remaining_walker
        .take_while(|_| has_deeper_entries.get())
        .map(|result| (false, result));
    let entries = priority_entries.chain(remaining_entries);
    for (priority_pass, result) in entries {
        if index.is_retired() {
            return;
        }
        let entry = match result {
            Ok(entry) if entry.depth() == 0 => continue,
            Ok(entry) => entry,
            Err(_) => {
                truncated = true;
                continue;
            }
        };
        if !priority_pass && entry.depth() <= priority_depth {
            continue;
        }
        if entry.depth() > max_depth {
            truncated = true;
            continue;
        }
        if indexed_entries >= max_entries || walk_start.elapsed() >= time_budget {
            truncated = true;
            break;
        }
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
        pending_items.push(SearchItem {
            name,
            is_directory,
            path,
            search_path,
            search_name_start,
            depth,
        });
        indexed_entries += 1;

        if pending_items.len() >= 256 {
            append_index_items(index, &mut pending_items);
        }
        if last_publish.elapsed() >= PUBLISH_INTERVAL {
            append_index_items(index, &mut pending_items);
            index.broadcast_change();
            last_publish = Instant::now();
        }
    }

    append_index_items(index, &mut pending_items);
    index.truncated.store(truncated, Ordering::Release);
    index.indexing.store(false, Ordering::Release);
    if truncated {
        tracing::warn!(
            entries = indexed_entries,
            elapsed_ms = walk_start.elapsed().as_millis() as u64,
            "search index truncated"
        );
    } else {
        tracing::info!(
            entries = indexed_entries,
            elapsed_ms = walk_start.elapsed().as_millis() as u64,
            "search index built"
        );
    }
    index.broadcast_change();
}

fn append_index_items(index: &SharedIndex, items: &mut Vec<SearchItem>) {
    if items.is_empty() {
        return;
    }
    index
        .items
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .append(items);
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

fn publish(sender: &Sender<SearchEvent>, progress: &WalkProgress, indexing: bool, truncated: bool) {
    let _sent = sender.send(SearchEvent::Results {
        query: progress.query.clone(),
        items: progress
            .matches
            .iter()
            .map(|(_, item)| item.clone())
            .collect(),
        indexing,
        truncated,
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
