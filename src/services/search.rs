// SPDX-License-Identifier: MIT

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

const RESULT_LIMIT: usize = 100;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

// Keep tool configuration searchable while pruning generated subtrees.
const GENERATED_TREE_GLOBS: [&str; 12] = [
    "!**/.cache/",
    "!**/.cargo/registry/",
    "!**/.cargo/git/",
    "!**/.rustup/downloads/",
    "!**/.gradle/caches/",
    "!**/.gradle/wrapper/dists/",
    "!**/.m2/repository/",
    "!**/.npm/_cacache/",
    "!**/.bun/install/cache/",
    "!**/node_modules/",
    "!**/target/",
    "!**/.venv/",
];

/// Bounds worst-case index memory on an adversarially large tree. Each retained `SearchItem`
/// and its traversal-wide deduplication key store full path strings, so cost scales with path
/// length, not just entry count: roughly 80-180 MB at this cap for typical paths, but up to
/// ~2.5 GB for paths near `PATH_MAX`.
const MAX_INDEX_ENTRIES: usize = 200_000;
const MAX_INDEX_DEPTH: usize = 64;
const INDEX_TIME_BUDGET: Duration = Duration::from_secs(10);
const INITIAL_DIRECTORY_BATCH: usize = 1;
const MAX_PENDING_DIRECTORIES: usize = 4_096;

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
    #[cfg(test)]
    pub(crate) fn for_test(path: PathBuf, is_directory: bool) -> Self {
        Self::new(
            path.clone(),
            path.parent().unwrap_or(Path::new("/")),
            is_directory,
        )
    }

    fn new(path: PathBuf, root: &Path, is_directory: bool) -> Self {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let depth = relative
            .components()
            .count()
            .saturating_sub(1)
            .min(MAX_INDEX_DEPTH) as u8;
        let search_path = relative.to_string_lossy().to_lowercase();
        let search_name_start = search_path
            .rfind(std::path::MAIN_SEPARATOR)
            .map_or(0, |position| {
                position + std::path::MAIN_SEPARATOR.len_utf8()
            });
        Self {
            name,
            path,
            is_directory,
            search_path,
            search_name_start,
            depth,
        }
    }

    fn search_name(&self) -> &str {
        &self.search_path[self.search_name_start..]
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchCoverage {
    pub entry_limit: bool,
    pub directory_limit: bool,
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
        if self.directory_limit {
            reasons.push("some folders were omitted");
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

struct IndexState {
    items: Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
}

struct SharedIndex {
    state: RwLock<IndexState>,
    subscribers: Mutex<Vec<(usize, Sender<SearchCommand>)>>,
    next_subscriber: AtomicUsize,
    lifecycle: Mutex<IndexLifecycle>,
}

impl SharedIndex {
    fn new() -> Self {
        Self {
            state: RwLock::new(IndexState {
                items: Vec::new(),
                indexing: true,
                coverage: SearchCoverage::default(),
            }),
            subscribers: Mutex::new(Vec::new()),
            next_subscriber: AtomicUsize::new(1),
            lifecycle: Mutex::new(IndexLifecycle {
                active_sessions: 1,
                retired: false,
            }),
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

mod directory;

type IndexRegistry = HashMap<(Vec<PathBuf>, bool, bool), Weak<SharedIndex>>;
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

pub fn index_tree(root: PathBuf, show_hidden: bool) -> (SearchHandle, Receiver<SearchEvent>) {
    index_trees(vec![root], show_hidden)
}

pub fn index_filter(
    root: PathBuf,
    show_hidden: bool,
    include_subfolders: bool,
) -> (SearchHandle, Receiver<SearchEvent>) {
    index_scoped(vec![root], show_hidden, include_subfolders)
}

/// Concurrent sessions share a snapshot until the last handle is dropped.
/// Indexing and scoring run off the GTK thread.
pub fn index_trees(
    roots: Vec<PathBuf>,
    show_hidden: bool,
) -> (SearchHandle, Receiver<SearchEvent>) {
    index_scoped(roots, show_hidden, true)
}

fn index_scoped(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    recursive: bool,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let mut seen = HashSet::new();
    let roots: Vec<_> = roots
        .into_iter()
        .filter(|root| seen.insert(root.clone()))
        .collect();
    let key = (roots.clone(), show_hidden, recursive);
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
            roots,
            show_hidden,
            MAX_INDEX_ENTRIES,
            MAX_INDEX_DEPTH,
            INDEX_TIME_BUDGET,
            recursive,
        );
        index
    };
    drop(registry);
    start_search_session(index)
}

#[cfg(test)]
fn index_trees_with_budget(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> (SearchHandle, Receiver<SearchEvent>) {
    index_trees_with_scheduler_budget(
        roots,
        show_hidden,
        max_entries,
        max_depth,
        time_budget,
        INITIAL_DIRECTORY_BATCH,
        MAX_PENDING_DIRECTORIES,
    )
}

#[cfg(test)]
fn index_trees_with_scheduler_budget(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    initial_directory_batch: usize,
    max_pending_directories: usize,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let index = Arc::new(SharedIndex::new());
    build_index(
        &index,
        roots,
        show_hidden,
        TraversalBudget {
            max_entries,
            max_depth,
            time_budget,
            initial_directory_batch,
            max_pending_directories,
        },
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
    let worker = std::thread::Builder::new()
        .name("strata-search-query".into())
        .spawn(move || {
            run_search_session(
                &worker_index,
                &worker_cancelled,
                &command_receiver,
                &event_sender,
            );
        });
    if let Err(error) = worker {
        tracing::error!(%error, "search query worker failed to start");
    }
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
        let state = index
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(query) = next_query {
            progress.normalized_query = query.to_lowercase();
            progress.query = query;
            progress.matches = if progress.normalized_query.is_empty() {
                Vec::new()
            } else {
                score_index(&state.items, &progress.normalized_query)
            };
        } else if index_changed && !progress.normalized_query.is_empty() {
            for item in &state.items[indexed_items..] {
                if let Some(score) = fuzzy_score_normalized(item, &progress.normalized_query) {
                    insert_match(&mut progress.matches, score, item);
                }
            }
        }
        indexed_items = state.items.len();
        // Completion must describe the same snapshot that was scored.
        let indexing = state.indexing;
        let coverage = state.coverage;
        drop(state);
        if query_changed || (index_changed && (!progress.query.is_empty() || !indexing)) {
            publish(events, &progress, indexing, coverage);
        }
    }
}

fn start_indexer(
    index: Arc<SharedIndex>,
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    recursive: bool,
) {
    let worker_index = index.clone();
    let worker = std::thread::Builder::new()
        .name("strata-search-index".into())
        .spawn(move || {
            if recursive {
                build_index(
                    &worker_index,
                    roots,
                    show_hidden,
                    TraversalBudget {
                        max_entries,
                        max_depth,
                        time_budget,
                        initial_directory_batch: INITIAL_DIRECTORY_BATCH,
                        max_pending_directories: MAX_PENDING_DIRECTORIES,
                    },
                );
            } else {
                directory::build_index(&worker_index, roots, show_hidden, max_entries, time_budget);
            }
        });
    if let Err(error) = worker {
        tracing::error!(%error, "search index worker failed to start");
        append_index_items(
            &index,
            &mut Vec::new(),
            false,
            SearchCoverage {
                unreadable: true,
                ..Default::default()
            },
        );
        index.broadcast_change();
    }
}

struct DirectoryTask {
    path: PathBuf,
    root: PathBuf,
    depth: usize,
    probe_only: bool,
    skipped_entries: usize,
    batch_size: usize,
    virtual_work: usize,
}

struct ScheduledDirectory {
    task: DirectoryTask,
    sequence: u64,
}

impl PartialEq for ScheduledDirectory {
    fn eq(&self, other: &Self) -> bool {
        self.task.virtual_work == other.task.virtual_work && self.sequence == other.sequence
    }
}

impl Eq for ScheduledDirectory {}

impl PartialOrd for ScheduledDirectory {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledDirectory {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .task
            .virtual_work
            .cmp(&self.task.virtual_work)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

fn directory_walker(
    task: &DirectoryTask,
    boundaries: Arc<HashSet<PathBuf>>,
    show_hidden: bool,
) -> ignore::Walk {
    let mut overrides = ignore::overrides::OverrideBuilder::new(&task.root);
    for generated_tree in GENERATED_TREE_GLOBS {
        overrides
            .add(generated_tree)
            .expect("valid generated-tree prune glob");
    }
    let mut builder = ignore::WalkBuilder::new(&task.path);
    builder
        .follow_links(false)
        .standard_filters(true)
        // `standard_filters` resets hidden-file filtering.
        .hidden(!show_hidden)
        .require_git(false)
        .overrides(overrides.build().expect("valid generated-tree prune globs"))
        .max_depth(Some(1))
        // Nested mounts are walked separately, never through both roots.
        .filter_entry(move |entry| entry.depth() == 0 || !boundaries.contains(entry.path()));
    builder.build()
}

struct TraversalBudget {
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    initial_directory_batch: usize,
    max_pending_directories: usize,
}

#[derive(Debug, Eq, PartialEq)]
enum PathAdmission {
    Unique,
    Duplicate,
    EntryLimit,
}

fn admit_path(
    indexed_paths: &mut HashSet<PathBuf>,
    path: &Path,
    max_entries: usize,
) -> PathAdmission {
    if indexed_paths.contains(path) {
        PathAdmission::Duplicate
    } else if indexed_paths.len() >= max_entries {
        PathAdmission::EntryLimit
    } else {
        indexed_paths.insert(path.to_path_buf());
        PathAdmission::Unique
    }
}

fn build_index(
    index: &SharedIndex,
    roots: Vec<PathBuf>,
    show_hidden: bool,
    budget: TraversalBudget,
) {
    let TraversalBudget {
        max_entries,
        max_depth,
        time_budget,
        initial_directory_batch,
        max_pending_directories,
    } = budget;
    let mut indexed_entries = 0;
    let mut indexed_paths = HashSet::new();
    let mut pending_items = Vec::with_capacity(256);
    let mut coverage = SearchCoverage::default();
    let mut last_publish = Instant::now();
    let walk_start = Instant::now();
    let mut seen = HashSet::new();
    let roots: Vec<_> = roots
        .into_iter()
        .filter(|root| seen.insert(root.clone()))
        .collect();
    let boundaries = Arc::new(seen);
    let initial_directory_batch = initial_directory_batch.max(1);
    let max_pending_directories = max_pending_directories.max(1);
    let mut pending_branches = VecDeque::new();
    let mut pending_directory_count = 0_usize;
    let mut next_sequence = 0_u64;
    for root in roots {
        if pending_directory_count >= max_pending_directories {
            coverage.directory_limit = true;
            continue;
        }
        let mut branch = BinaryHeap::new();
        branch.push(ScheduledDirectory {
            task: DirectoryTask {
                path: root.clone(),
                root,
                depth: 0,
                probe_only: max_depth == 0,
                skipped_entries: 0,
                batch_size: initial_directory_batch,
                virtual_work: 0,
            },
            sequence: next_sequence,
        });
        next_sequence = next_sequence.wrapping_add(1);
        pending_branches.push_back(branch);
        pending_directory_count += 1;
    }

    'walk: while let Some(mut branch) = pending_branches.pop_front() {
        let Some(scheduled) = branch.pop() else {
            continue;
        };
        pending_directory_count = pending_directory_count.saturating_sub(1);
        let mut directory = scheduled.task;
        let mut new_branches = Vec::new();
        if index.is_retired() {
            return;
        }
        let mut walker = directory_walker(&directory, boundaries.clone(), show_hidden);
        let mut seen_entries = 0;
        let mut processed_entries = 0;
        let mut slice_work = 0_usize;
        let exhausted = loop {
            if index.is_retired() {
                return;
            }
            if walk_start.elapsed() >= time_budget {
                coverage.time_limit = true;
                break 'walk;
            }
            if processed_entries >= directory.batch_size {
                break false;
            }
            let Some(result) = walker.next() else {
                break true;
            };
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    coverage.unreadable = true;
                    continue;
                }
            };
            if entry.depth() == 0 {
                if entry.error().is_some() {
                    coverage.unreadable = true;
                }
                continue;
            }
            if seen_entries < directory.skipped_entries {
                seen_entries += 1;
                continue;
            }
            seen_entries += 1;
            processed_entries += 1;
            if entry.error().is_some() {
                coverage.unreadable = true;
            }
            let is_directory = entry.file_type().is_some_and(|kind| kind.is_dir());
            // Structural entries are cheap within a branch so nested documents progress
            // before dense runs of regular files consume the shared entry budget.
            slice_work = slice_work.saturating_add(if is_directory { 1 } else { 8 });
            if directory.probe_only {
                coverage.depth_limit = true;
                continue;
            }
            let path = entry.into_path();
            match admit_path(&mut indexed_paths, &path, max_entries) {
                PathAdmission::Duplicate => continue,
                PathAdmission::EntryLimit => {
                    coverage.entry_limit = true;
                    break 'walk;
                }
                PathAdmission::Unique => {}
            }
            let entry_depth = directory.depth.saturating_add(1);
            pending_items.push(SearchItem::new(path.clone(), &directory.root, is_directory));
            indexed_entries += 1;
            if is_directory {
                // Keep one queue slot available for this slice's continuation. A child that
                // cannot be admitted is omitted, while already queued work still completes.
                if pending_directory_count < max_pending_directories.saturating_sub(1) {
                    let child = ScheduledDirectory {
                        task: DirectoryTask {
                            path,
                            root: directory.root.clone(),
                            depth: entry_depth,
                            probe_only: entry_depth >= max_depth,
                            skipped_entries: 0,
                            batch_size: initial_directory_batch,
                            virtual_work: directory.virtual_work.saturating_add(slice_work),
                        },
                        sequence: next_sequence,
                    };
                    next_sequence = next_sequence.wrapping_add(1);
                    pending_directory_count += 1;
                    if directory.depth == 0 {
                        let mut child_branch = BinaryHeap::new();
                        child_branch.push(child);
                        new_branches.push(child_branch);
                    } else {
                        branch.push(child);
                    }
                } else {
                    coverage.directory_limit = true;
                }
            }
            if pending_items.len() >= 256 {
                append_index_items(index, &mut pending_items, true, coverage);
            }
            if last_publish.elapsed() >= PUBLISH_INTERVAL {
                append_index_items(index, &mut pending_items, true, coverage);
                index.broadcast_change();
                last_publish = Instant::now();
            }
        };
        drop(walker);
        if !exhausted {
            directory.skipped_entries = directory.skipped_entries.saturating_add(processed_entries);
            directory.batch_size = directory.batch_size.saturating_mul(2);
            directory.virtual_work = directory.virtual_work.saturating_add(slice_work);
            branch.push(ScheduledDirectory {
                task: directory,
                sequence: next_sequence,
            });
            next_sequence = next_sequence.wrapping_add(1);
            pending_directory_count += 1;
        }
        if !branch.is_empty() {
            pending_branches.push_back(branch);
        }
        pending_branches.extend(new_branches);
    }
    append_index_items(index, &mut pending_items, false, coverage);
    if coverage.is_partial() {
        tracing::warn!(
            entries = indexed_entries,
            elapsed_ms = walk_start.elapsed().as_millis() as u64,
            ?coverage,
            "search index partial"
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

fn append_index_items(
    index: &SharedIndex,
    items: &mut Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let mut state = index
        .state
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.items.append(items);
    state.indexing = indexing;
    state.coverage = coverage;
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
                    scope.spawn(move || score_range(items, normalized_query, chunk * chunk_size))
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
        let Some(score) = fuzzy_score_normalized(item, normalized_query) else {
            continue;
        };
        retain_candidate(&mut best, (score, Reverse(position_offset + position)));
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
    let position = matches.partition_point(|candidate| candidate.0 >= score);
    if position < RESULT_LIMIT {
        matches.insert(position, (score, item.clone()));
        matches.truncate(RESULT_LIMIT);
    }
}

fn publish(
    sender: &Sender<SearchEvent>,
    progress: &WalkProgress,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let _sent = sender.send(SearchEvent::Results {
        query: progress.query.clone(),
        items: progress
            .matches
            .iter()
            .map(|(_, item)| item.clone())
            .collect(),
        indexing,
        coverage,
    });
}

fn fuzzy_score_normalized(item: &SearchItem, query: &str) -> Option<i64> {
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
    score -= i64::from(item.depth) * 32;
    Some(score)
}

fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
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

#[cfg(test)]
mod tests;
