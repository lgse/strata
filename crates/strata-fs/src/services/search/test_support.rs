// SPDX-License-Identifier: MIT
//! Index entry points with explicit budgets, for the search tests.

use std::{
    path::PathBuf,
    sync::{Arc, mpsc::Receiver},
    time::Duration,
};

use super::{
    INITIAL_DIRECTORY_BATCH, MAX_PENDING_DIRECTORIES, SearchEvent, SearchExclusions, SearchHandle,
    SearchLimits, SharedIndex, TraversalBudget, index_trees_with_exclusions, open_index,
    refresh::{RefreshMode, RefreshTarget, request_refresh},
    scoring::{SearchScorer, fuzzy_score_normalized},
    session::start_search_session,
    walk::build_index,
};

pub fn index_tree(root: PathBuf, show_hidden: bool) -> (SearchHandle, Receiver<SearchEvent>) {
    index_trees(vec![root], show_hidden)
}

pub fn index_trees(
    roots: Vec<PathBuf>,
    show_hidden: bool,
) -> (SearchHandle, Receiver<SearchEvent>) {
    index_trees_with_exclusions(roots, show_hidden, Vec::new(), SearchLimits::default())
}

fn standard_scheduler_budget(
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> TraversalBudget {
    TraversalBudget {
        max_entries,
        max_depth,
        time_budget,
        initial_directory_batch: INITIAL_DIRECTORY_BATCH,
        max_pending_directories: MAX_PENDING_DIRECTORIES,
    }
}

pub(super) fn index_trees_with_budget(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let budget = standard_scheduler_budget(max_entries, max_depth, time_budget);
    index_trees_with_scheduler_budget_and_exclusions(
        roots,
        show_hidden,
        budget,
        SearchExclusions::default(),
    )
}

pub(super) fn index_trees_with_scheduler_budget(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    initial_directory_batch: usize,
    max_pending_directories: usize,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let budget = TraversalBudget {
        initial_directory_batch,
        max_pending_directories,
        ..standard_scheduler_budget(max_entries, max_depth, time_budget)
    };
    index_trees_with_scheduler_budget_and_exclusions(
        roots,
        show_hidden,
        budget,
        SearchExclusions::default(),
    )
}

pub(super) fn index_trees_with_scheduler_budget_and_exclusions(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    budget: TraversalBudget,
    exclusions: SearchExclusions,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let index = Arc::new(SharedIndex::new());
    build_index(&index, roots, show_hidden, budget, exclusions);
    start_search_session(index, SearchScorer::Name(fuzzy_score_normalized))
}

pub(crate) fn index_trees_with_budget_and_exclusions(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    exclusions: Vec<String>,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let budget = standard_scheduler_budget(max_entries, max_depth, time_budget);
    index_trees_with_scheduler_budget_and_exclusions(
        roots,
        show_hidden,
        budget,
        SearchExclusions::from_strings(&exclusions),
    )
}

pub(super) fn request_index_refresh(
    index: Arc<SharedIndex>,
    roots: Vec<PathBuf>,
    hidden: bool,
    recursive: bool,
    exclusions: SearchExclusions,
    mode: RefreshMode,
) {
    let target = RefreshTarget {
        roots,
        hidden,
        recursive,
        exclusions,
    };
    request_refresh(index, target, mode);
}

pub(super) fn index_scoped(
    roots: Vec<PathBuf>,
    show_hidden: bool,
    recursive: bool,
    scorer: SearchScorer,
    exclusions: Vec<String>,
    limits: SearchLimits,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let target = RefreshTarget::new(roots, show_hidden, recursive, &exclusions);
    open_index(target, limits, scorer)
}
