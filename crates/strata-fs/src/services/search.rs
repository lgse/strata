// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
};

use crate::model::{EntryKind, MetadataValue};
use unicode_normalization::UnicodeNormalization;

use super::{
    file_activity, is_hidden_name, native_hidden_names, native_kind,
    path_match::{self, Frecency, PathMatcher, TextScore},
};

pub const RESULT_LIMIT: usize = 100;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);
/// The shortest gap between the starts of two rescans of one index for outside changes.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

// Keep tool configuration searchable while pruning generated subtrees.
const GENERATED_TREE_GLOBS: [&str; 14] = [
    "!**/.cache/",
    "!**/.cargo/registry/",
    "!**/.cargo/git/",
    "!**/.rustup/downloads/",
    "!**/.gradle/caches/",
    "!**/.gradle/wrapper/dists/",
    "!**/.m2/repository/",
    "!**/.npm/_cacache/",
    "!**/.bun/install/cache/",
    "!**/go/pkg/mod/",
    "!**/node_modules/",
    "!**/target/",
    "!**/.venv/",
    "!**/__pycache__/",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TraversalBudget {
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    initial_directory_batch: usize,
    max_pending_directories: usize,
}

impl TraversalBudget {
    const fn standard() -> Self {
        Self {
            max_entries: MAX_INDEX_ENTRIES,
            max_depth: MAX_INDEX_DEPTH,
            time_budget: INDEX_TIME_BUDGET,
            initial_directory_batch: INITIAL_DIRECTORY_BATCH,
            max_pending_directories: MAX_PENDING_DIRECTORIES,
        }
    }

    fn with_limits(limits: SearchLimits) -> Self {
        Self {
            max_entries: limits.max_entries.unwrap_or(usize::MAX),
            time_budget: limits.time_budget.unwrap_or(Duration::MAX),
            ..Self::standard()
        }
    }
}

/// How far one index walk may go. The defaults keep local searches responsive and bound
/// index memory; slow or network mounts may need more of both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchLimits {
    /// `None` walks until every root is exhausted.
    pub time_budget: Option<Duration>,
    /// `None` keeps every entry; memory grows with the tree.
    pub max_entries: Option<usize>,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            time_budget: Some(INDEX_TIME_BUDGET),
            max_entries: Some(MAX_INDEX_ENTRIES),
        }
    }
}

pub fn fold_for_search(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    text.to_lowercase().nfc().collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchItem {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
    pub kind: EntryKind,
    pub mode: MetadataValue<u32>,
    search_path: String,
    search_name_start: usize,
    depth: u8,
}

impl SearchItem {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(path: PathBuf, is_directory: bool) -> Self {
        Self::new(
            path.clone(),
            path.parent().unwrap_or(Path::new("/")),
            is_directory,
        )
    }

    pub(super) fn for_history(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let search_path = fold_for_search(&path.to_string_lossy());
        let search_name_start = search_path
            .rfind(std::path::MAIN_SEPARATOR)
            .map_or(0, |position| {
                position + std::path::MAIN_SEPARATOR.len_utf8()
            });
        Self {
            path,
            name,
            is_directory: true,
            kind: EntryKind::Directory,
            mode: MetadataValue::Unknown,
            search_path,
            search_name_start,
            depth: 0,
        }
    }

    pub(super) fn path_score(&self, matcher: &mut PathMatcher) -> Option<TextScore> {
        matcher.score(&self.search_path, self.search_name_start)
    }

    pub(super) fn name_is(&self, normalized_name: &str) -> bool {
        self.search_name() == normalized_name
    }

    #[cfg(any(test, feature = "test-support"))]
    fn new(path: PathBuf, root: &Path, is_directory: bool) -> Self {
        let kind = if is_directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        Self::with_metadata(path, root, is_directory, kind, MetadataValue::Unknown)
    }

    fn with_metadata(
        path: PathBuf,
        root: &Path,
        is_directory: bool,
        kind: EntryKind,
        mode: MetadataValue<u32>,
    ) -> Self {
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
        let search_path = fold_for_search(&relative.to_string_lossy());
        let search_name_start = search_path
            .rfind(std::path::MAIN_SEPARATOR)
            .map_or(0, |position| {
                position + std::path::MAIN_SEPARATOR.len_utf8()
            });
        Self {
            name,
            path,
            is_directory,
            kind,
            mode,
            search_path,
            search_name_start,
            depth,
        }
    }

    fn search_name(&self) -> &str {
        &self.search_path[self.search_name_start..]
    }

    /// This result after a rename within its folder to `path`.
    pub fn renamed(&self, path: PathBuf) -> Self {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let search_path = format!(
            "{}{}",
            &self.search_path[..self.search_name_start],
            fold_for_search(&name)
        );
        Self {
            name,
            path,
            search_path,
            ..self.clone()
        }
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

    /// Localized status text; empty when the search was complete.
    pub fn message(self) -> String {
        let reasons = [
            (self.entry_limit, "entry limit reached"),
            (self.directory_limit, "some folders were omitted"),
            (self.depth_limit, "depth limit reached"),
            (self.time_limit, "indexing time limit reached"),
            (self.unreadable, "some folders could not be read"),
        ]
        .into_iter()
        .filter(|(applies, _)| *applies)
        .map(|(_, reason)| crate::i18n::tr(reason))
        .reduce(|first, second| {
            rust_i18n::t!("%{first}; %{second}", first = first, second = second).into_owned()
        });
        reasons.map_or_else(String::new, |reasons| {
            let message =
                rust_i18n::t!("Partial search — %{reasons}", reasons = reasons).into_owned();
            if self.entry_limit || self.time_limit {
                crate::i18n::join_sentences(
                    &message,
                    &crate::i18n::tr(
                        "Raise the limits in Settings → General → Search & filtering.",
                    ),
                )
            } else {
                message
            }
        })
    }
}

pub enum SearchEvent {
    Results {
        query: String,
        items: Vec<SearchItem>,
        indexing: bool,
        coverage: SearchCoverage,
        has_more: bool,
        candidates: usize,
    },
}

enum SearchCommand {
    Query(String, usize),
    IndexChanged,
}

#[derive(Default)]
struct WalkProgress {
    query: String,
    normalized_query: String,
    matches: Vec<(i64, SearchItem)>,
    limit: usize,
}

struct IndexLifecycle {
    active_sessions: usize,
    retired: bool,
}

struct IndexState {
    revision: usize,
    items: Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
}

struct SharedIndex {
    state: RwLock<IndexState>,
    subscribers: Mutex<Vec<(usize, Sender<SearchCommand>)>>,
    next_subscriber: AtomicUsize,
    lifecycle: Mutex<IndexLifecycle>,
    cancel_initial_indexer: AtomicBool,
    refresh_requested: AtomicUsize,
    refresh: Mutex<RefreshState>,
    refresh_owner: Option<(Weak<SharedIndex>, usize)>,
    /// `build_root_overrides` per root, kept for the scope checks of outside changes.
    walk_overrides: Mutex<HashMap<PathBuf, Arc<ignore::overrides::Override>>>,
}

struct RefreshState {
    running: bool,
    target: RefreshTarget,
    budget: TraversalBudget,
    /// A rescan arrived while a walk ran: walk once more after it publishes.
    rerun: bool,
    /// The next walk only rescans unchanged roots, so it waits out `MIN_REFRESH_INTERVAL`.
    throttled: bool,
    last_walk: Option<Instant>,
}

impl SharedIndex {
    fn new() -> Self {
        Self {
            state: RwLock::new(IndexState {
                revision: 0,
                items: Vec::new(),
                indexing: true,
                coverage: SearchCoverage::default(),
            }),
            subscribers: Mutex::new(Vec::new()),
            next_subscriber: AtomicUsize::new(1),
            cancel_initial_indexer: AtomicBool::new(false),
            refresh_requested: AtomicUsize::new(0),
            refresh: Mutex::new(RefreshState {
                running: false,
                target: RefreshTarget::default(),
                budget: TraversalBudget::standard(),
                rerun: false,
                throttled: false,
                last_walk: None,
            }),
            refresh_owner: None,
            walk_overrides: Mutex::new(HashMap::new()),
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

    fn indexing_cancelled(&self) -> bool {
        self.cancel_initial_indexer.load(Ordering::Acquire)
            || self.is_retired()
            || self
                .refresh_owner
                .as_ref()
                .is_some_and(|(owner, generation)| {
                    owner.upgrade().is_none_or(|owner| {
                        owner.is_retired()
                            || owner.refresh_requested.load(Ordering::Acquire) != *generation
                    })
                })
    }

    fn walk_overrides(&self, root: &Path) -> Arc<ignore::overrides::Override> {
        self.walk_overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(build_root_overrides(root)))
            .clone()
    }

    fn is_retired(&self) -> bool {
        self.lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retired
    }
}

mod directory;
pub(crate) mod exclusions;
mod pattern;
mod refresh;
mod scoring;
mod session;
#[cfg(test)]
mod test_support;
mod walk;

pub use exclusions::SearchExclusions;
pub use pattern::{filter_name_matches, filter_query_allows_typos};
#[cfg(test)]
use refresh::RefreshMode;
use refresh::{RefreshTarget, start_indexer};
pub use refresh::{
    RenameScope, rebase_search_indexes, refresh_search_indexes_for_directory,
    refresh_search_indexes_for_rename,
};
pub use scoring::RefusedFolders;
use scoring::{SearchScorer, filter_score_normalized, fuzzy_score_normalized};
#[cfg(test)]
use scoring::{fuzzy_subsequence_score, insert_match, score_index};
use session::start_search_session;
#[cfg(test)]
use test_support::{
    index_scoped, index_tree, index_trees, index_trees_with_budget,
    index_trees_with_budget_and_exclusions, index_trees_with_scheduler_budget,
    index_trees_with_scheduler_budget_and_exclusions, request_index_refresh,
};
#[cfg(test)]
use walk::{PathAdmission, admit_path};
use walk::{append_index_items, build_root_overrides};

/// Indexes are shared by sessions that walk the same roots the same way.
#[derive(Clone, PartialEq)]
struct IndexKey {
    target: RefreshTarget,
    limits: SearchLimits,
}

type IndexRegistry = Vec<(IndexKey, Weak<SharedIndex>)>;
static SHARED_INDEXES: OnceLock<Mutex<IndexRegistry>> = OnceLock::new();
static REFRESH_TRAVERSAL: Mutex<()> = Mutex::new(());

fn lock_registry(registry: &Mutex<IndexRegistry>) -> MutexGuard<'_, IndexRegistry> {
    let mut registry = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|(_, index)| index.strong_count() > 0);
    registry
}

fn unique_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    roots
        .into_iter()
        .filter(|root| seen.insert(root.clone()))
        .collect()
}

pub struct SearchHandle {
    cancelled: Arc<AtomicBool>,
    commands: Sender<SearchCommand>,
    index: Arc<SharedIndex>,
    subscriber_id: usize,
}

impl SearchHandle {
    pub fn query(&self, query: &str) {
        self.query_candidates(query, RESULT_LIMIT);
    }

    pub fn query_candidates(&self, query: &str, limit: usize) {
        let _sent = self.commands.send(SearchCommand::Query(
            query.trim().to_owned(),
            limit.clamp(1, MAX_INDEX_ENTRIES),
        ));
    }
}

impl SearchHandle {
    #[cfg(any(test, feature = "test-support"))]
    pub fn shares_index_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.index, &other.index)
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

pub fn index_filter(
    root: PathBuf,
    show_hidden: bool,
    include_subfolders: bool,
    limits: SearchLimits,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let target = RefreshTarget::new(vec![root], show_hidden, include_subfolders, &[]);
    open_index(target, limits, SearchScorer::Name(filter_score_normalized))
}

pub fn index_paths(
    root: PathBuf,
    show_hidden: bool,
    recursive: bool,
    frecency: Frecency,
    limits: SearchLimits,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let scorer = SearchScorer::Paths {
        frecency: Arc::new(frecency),
        folders: None,
    };
    let target = RefreshTarget::new(vec![root], show_hidden, recursive, &[]);
    open_index(target, limits, scorer)
}

pub fn index_folder_paths(
    root: PathBuf,
    show_hidden: bool,
    frecency: Frecency,
    refused: RefusedFolders,
    limits: SearchLimits,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let scorer = SearchScorer::Paths {
        frecency: Arc::new(frecency),
        folders: Some(Arc::new(refused)),
    };
    open_index(
        RefreshTarget::new(vec![root], show_hidden, true, &[]),
        limits,
        scorer,
    )
}

/// Concurrent sessions share a snapshot until the last handle is dropped.
/// Indexing and scoring run off the GTK thread.
pub fn index_trees_with_exclusions(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    exclusions: Vec<String>,
    limits: SearchLimits,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let target = RefreshTarget::new(roots, show_hidden, true, &exclusions);
    open_index(target, limits, SearchScorer::Name(fuzzy_score_normalized))
}

/// A live index for `key`, joining a running one when another session shares it.
fn shared_index(key: IndexKey) -> Arc<SharedIndex> {
    let registry = SHARED_INDEXES.get_or_init(|| Mutex::new(Vec::new()));
    let mut registry = lock_registry(registry);
    let shared = registry
        .iter()
        .filter(|(candidate, _)| candidate == &key)
        .filter_map(|(_, index)| index.upgrade())
        .find(|index| index.try_acquire());
    if let Some(index) = shared {
        return index;
    }
    let index = Arc::new(SharedIndex::new());
    registry.push((key.clone(), Arc::downgrade(&index)));
    start_indexer(
        index.clone(),
        key.target,
        TraversalBudget::with_limits(key.limits),
    );
    index
}

fn open_index(
    target: RefreshTarget,
    limits: SearchLimits,
    scorer: SearchScorer,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let index = shared_index(IndexKey { target, limits });
    start_search_session(index, scorer)
}

#[cfg(test)]
mod tests;
