// SPDX-License-Identifier: MIT
//! Rescanning shared indexes after outside changes, renames and moves.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use super::{
    MAX_INDEX_DEPTH, MIN_REFRESH_INTERVAL, REFRESH_TRAVERSAL, SHARED_INDEXES, SearchCoverage,
    SearchExclusions, SharedIndex, TraversalBudget, directory, is_hidden_name, lock_registry,
    unique_roots,
    walk::{append_index_items, build_index},
};

/// The roots one index walks and how it walks them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RefreshTarget {
    pub(super) roots: Vec<PathBuf>,
    pub(super) hidden: bool,
    pub(super) recursive: bool,
    pub(super) exclusions: SearchExclusions,
}

impl RefreshTarget {
    pub(super) fn new(
        roots: Vec<PathBuf>,
        hidden: bool,
        recursive: bool,
        exclusions: &[String],
    ) -> Self {
        Self {
            roots: unique_roots(roots),
            hidden,
            recursive,
            exclusions: SearchExclusions::from_strings(exclusions),
        }
    }

    fn component_listed(
        &self,
        overrides: &ignore::overrides::Override,
        path: &Path,
        name: &std::ffi::OsStr,
    ) -> bool {
        let shown = self.hidden || !is_hidden_name(name, &HashSet::new());
        shown && !overrides.matched(path, true).is_ignore()
    }

    /// Whether an index walk of `root` lists the entries of `directory`. A flat walk lists
    /// only `root`; a recursive one skips what `build_index` prunes: excluded and generated
    /// folders, hidden ones unless `hidden`, and folders at `MAX_INDEX_DEPTH` or deeper.
    fn walk_lists_directory(&self, index: &SharedIndex, root: &Path, directory: &Path) -> bool {
        if !self.recursive {
            return directory == root;
        }
        let Ok(relative) = directory.strip_prefix(root) else {
            return false;
        };
        if relative.components().count() >= MAX_INDEX_DEPTH {
            return false;
        }
        let name = directory.file_name().unwrap_or_default().to_string_lossy();
        if self.exclusions.is_excluded(directory, &name, true) {
            return false;
        }
        let overrides = index.walk_overrides(root);
        let mut path = root.to_path_buf();
        relative.components().all(|component| {
            path.push(component);
            self.component_listed(&overrides, &path, component.as_os_str())
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameScope {
    /// Every index with a root at, above or below either path.
    Related,
    /// Only indexes with a root at or below `from`, which move to `to`. Callers rescan
    /// the folder that held the entry themselves, so a plain file rename coalesces.
    Moved,
}

impl RenameScope {
    fn affects(self, root: &Path, from: &Path, to: &Path) -> bool {
        match self {
            Self::Related => {
                let renamed_inside = from.starts_with(root) || to.starts_with(root);
                let renamed_above = root.starts_with(from) || root.starts_with(to);
                renamed_inside || renamed_above
            }
            Self::Moved => root.starts_with(from),
        }
    }
}

fn rebase_roots(roots: &mut Vec<PathBuf>, from: &Path, to: &Path) {
    for root in roots.iter_mut() {
        if let Ok(suffix) = root.strip_prefix(from) {
            *root = to.join(suffix);
        }
    }
    *roots = unique_roots(std::mem::take(roots));
}

pub fn rebase_search_indexes(from: &Path, to: &Path, scope: RenameScope) {
    let Some(registry) = SHARED_INDEXES.get() else {
        return;
    };
    let mut registry = lock_registry(registry);
    for (key, index) in registry.iter_mut() {
        let target = &mut key.target;
        if !target
            .roots
            .iter()
            .any(|root| scope.affects(root, from, to))
        {
            continue;
        }
        rebase_roots(&mut target.roots, from, to);
        let Some(index) = index.upgrade() else {
            continue;
        };
        index
            .walk_overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|root, _| target.roots.contains(root));
        request_refresh(index, target.clone(), RefreshMode::Restart);
    }
}

pub fn refresh_search_indexes_for_rename(from: &Path, to: &Path) {
    rebase_search_indexes(from, to, RenameScope::Related);
}

/// Rescans, coalescing, every live index whose walk lists the entries of `directory`.
pub fn refresh_search_indexes_for_directory(directory: &Path) {
    let Some(registry) = SHARED_INDEXES.get() else {
        return;
    };
    let registry = lock_registry(registry);
    for (key, index) in registry.iter() {
        let Some(index) = index.upgrade() else {
            continue;
        };
        let target = &key.target;
        if target
            .roots
            .iter()
            .any(|root| target.walk_lists_directory(&index, root, directory))
        {
            request_refresh(index, target.clone(), RefreshMode::Coalesce);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefreshMode {
    /// The roots may have changed: cancel a running walk and start over at once.
    Restart,
    /// The same roots changed on disk: let a running walk publish, then walk once more,
    /// at most once per `MIN_REFRESH_INTERVAL`.
    Coalesce,
}

fn spawn_refresh_worker(index: &Arc<SharedIndex>) -> bool {
    for attempt in 0..2 {
        let worker_index = index.clone();
        match std::thread::Builder::new()
            .name("strata-search-refresh".into())
            .spawn(move || refresh_index(&worker_index))
        {
            Ok(_) => return true,
            Err(error) => tracing::error!(%error, attempt, "search refresh worker failed to start"),
        }
    }
    false
}

fn abandon_refresh(index: &SharedIndex) {
    let mut refresh = index
        .refresh
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    refresh.running = false;
    let mut state = index
        .state
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.coverage.unreadable = true;
    drop(state);
    index.broadcast_change();
}

pub(super) fn request_refresh(index: Arc<SharedIndex>, target: RefreshTarget, mode: RefreshMode) {
    let mut refresh = index
        .refresh
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if mode == RefreshMode::Coalesce && refresh.running {
        refresh.rerun = true;
        return;
    }
    index.refresh_requested.fetch_add(1, Ordering::AcqRel);
    refresh.target = target;
    refresh.throttled = mode == RefreshMode::Coalesce;
    if refresh.running {
        return;
    }
    refresh.running = true;
    refresh.rerun = false;
    drop(refresh);
    if !spawn_refresh_worker(&index) {
        abandon_refresh(&index);
    }
}

/// Claims the next walk, or `None` once the index retired.
fn begin_refresh_walk(index: &SharedIndex) -> Option<(usize, RefreshTarget, TraversalBudget)> {
    let mut refresh = index
        .refresh
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if index.is_retired() {
        refresh.running = false;
        return None;
    }
    refresh.last_walk = Some(Instant::now());
    // This walk already lists every change requested so far.
    refresh.rerun = false;
    Some((
        index.refresh_requested.load(Ordering::Acquire),
        refresh.target.clone(),
        refresh.budget,
    ))
}

fn adopt_replacement(index: &SharedIndex, replacement: SharedIndex) {
    let fresh = replacement
        .state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut state = index
        .state
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let revision = state.revision.wrapping_add(1);
    *state = fresh;
    state.revision = revision;
    drop(state);
    index.broadcast_change();
}

fn refresh_index(index: &Arc<SharedIndex>) {
    loop {
        if !wait_out_refresh_interval(index) {
            return;
        }
        // Bound replacement memory/traversal across overlapping scopes, not just per index.
        let traversal = REFRESH_TRAVERSAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some((generation, target, budget)) = begin_refresh_walk(index) else {
            return;
        };
        index.cancel_initial_indexer.store(true, Ordering::Release);
        let replacement = SharedIndex {
            refresh_owner: Some((Arc::downgrade(index), generation)),
            ..SharedIndex::new()
        };
        walk_roots(&replacement, target, budget);
        let mut refresh = index
            .refresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if index.is_retired() {
            refresh.running = false;
            return;
        }
        if index.refresh_requested.load(Ordering::Acquire) == generation {
            adopt_replacement(index, replacement);
            if refresh.rerun {
                refresh.throttled = true;
                continue;
            }
            refresh.running = false;
            return;
        }
        drop(refresh);
        drop(traversal);
    }
}

/// Sleeps until `MIN_REFRESH_INTERVAL` after the previous walk started, unless a
/// `RefreshMode::Restart` ends the wait. False once the index retired.
fn wait_out_refresh_interval(index: &SharedIndex) -> bool {
    const SLICE: Duration = Duration::from_millis(20);
    loop {
        let wait = {
            let mut refresh = index
                .refresh
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if index.is_retired() {
                refresh.running = false;
                return false;
            }
            refresh
                .last_walk
                .filter(|_| refresh.throttled)
                .map_or(Duration::ZERO, |started| {
                    MIN_REFRESH_INTERVAL.saturating_sub(started.elapsed())
                })
        };
        if wait.is_zero() {
            return true;
        }
        std::thread::sleep(wait.min(SLICE));
    }
}

/// Walks every root of `target` into `index`, recursively or one level deep.
fn walk_roots(index: &SharedIndex, target: RefreshTarget, budget: TraversalBudget) {
    let RefreshTarget {
        roots,
        hidden,
        recursive,
        exclusions,
    } = target;
    if recursive {
        build_index(index, roots, hidden, budget, exclusions);
    } else {
        directory::build_index(
            index,
            roots,
            hidden,
            budget.max_entries,
            budget.time_budget,
            &exclusions,
        );
    }
}

pub(super) fn start_indexer(
    index: Arc<SharedIndex>,
    target: RefreshTarget,
    budget: TraversalBudget,
) {
    index
        .refresh
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .budget = budget;
    let worker_index = index.clone();
    let worker = std::thread::Builder::new()
        .name("strata-search-index".into())
        .spawn(move || walk_roots(&worker_index, target, budget));
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
