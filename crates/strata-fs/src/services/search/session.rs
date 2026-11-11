// SPDX-License-Identifier: MIT
//! The query worker behind each `SearchHandle`.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::Duration,
};

use super::{
    IndexState, MAX_INDEX_ENTRIES, RESULT_LIMIT, SearchCommand, SearchCoverage, SearchEvent,
    SearchHandle, SearchItem, SharedIndex, WalkProgress, fold_for_search,
    scoring::{SearchScorer, insert_match_with_limit, score_index_with_limit},
};

pub(super) fn start_search_session(
    index: Arc<SharedIndex>,
    scorer: SearchScorer,
) -> (SearchHandle, Receiver<SearchEvent>) {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let subscriber_id = index.subscribe(command_sender.clone());
    let worker_cancelled = cancelled.clone();
    let worker_index = index.clone();
    let worker = std::thread::Builder::new()
        .name("strata-search-query".into())
        .spawn(move || {
            let worker = QueryWorker::new(&worker_index, &event_sender, &scorer);
            run_search_session(worker, &worker_cancelled, &command_receiver);
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

/// The newest query among queued commands, and whether the index changed meanwhile.
fn drain_commands(
    first: SearchCommand,
    commands: &Receiver<SearchCommand>,
) -> (Option<(String, usize)>, bool) {
    let mut next_query = None;
    let mut index_changed = false;
    for command in std::iter::once(first).chain(commands.try_iter()) {
        match command {
            SearchCommand::Query(query, limit) => next_query = Some((query, limit)),
            SearchCommand::IndexChanged => index_changed = true,
        }
    }
    (next_query, index_changed)
}

/// Scores one session's query against the shared index as either changes.
struct QueryWorker<'a> {
    index: &'a SharedIndex,
    events: &'a Sender<SearchEvent>,
    scorer: &'a SearchScorer,
    progress: WalkProgress,
    indexed_items: usize,
    revision: usize,
}

impl<'a> QueryWorker<'a> {
    fn new(
        index: &'a SharedIndex,
        events: &'a Sender<SearchEvent>,
        scorer: &'a SearchScorer,
    ) -> Self {
        Self {
            index,
            events,
            scorer,
            progress: WalkProgress::default(),
            indexed_items: 0,
            revision: 0,
        }
    }

    fn rescore(&mut self, items: &[SearchItem], lists: bool) {
        self.progress.matches = if !lists {
            Vec::new()
        } else {
            score_index_with_limit(
                items,
                &self.progress.normalized_query,
                self.scorer,
                self.progress.limit,
            )
        };
    }

    fn score_new_items(&mut self, items: &[SearchItem]) {
        let mut scorer = self.scorer.prepare(&self.progress.normalized_query);
        for item in &items[self.indexed_items..] {
            if let Some(score) = scorer.score(item) {
                insert_match_with_limit(
                    &mut self.progress.matches,
                    score,
                    item,
                    self.progress.limit,
                );
            }
        }
    }

    fn lists(&self) -> bool {
        !self.progress.normalized_query.is_empty() || self.scorer.lists_without_query()
    }

    /// Rescores when the query or snapshot changed, otherwise scores only appended items.
    fn update_matches(&mut self, state: &IndexState, query_changed: bool, index_changed: bool) {
        let lists = self.lists();
        let stale = query_changed || self.revision != state.revision;
        let extends_beyond_first_page = index_changed && self.progress.limit > RESULT_LIMIT;
        if stale || extends_beyond_first_page {
            self.rescore(&state.items, lists);
        } else if index_changed && lists {
            self.score_new_items(&state.items);
        }
        self.revision = state.revision;
        self.indexed_items = state.items.len();
    }

    fn handle(&mut self, next_query: Option<(String, usize)>, index_changed: bool) {
        let query_changed = next_query.is_some();
        let index = self.index;
        let state = index
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((query, limit)) = next_query {
            self.progress.normalized_query = fold_for_search(&query);
            self.progress.query = query;
            self.progress.limit = limit;
        }
        self.update_matches(&state, query_changed, index_changed);
        // Completion must describe the same snapshot that was scored.
        let indexing = state.indexing;
        let coverage = state.coverage;
        drop(state);
        let visible_change = self.lists() || !indexing;
        let announces_index = index_changed && visible_change;
        if query_changed || announces_index {
            publish(self.events, &self.progress, indexing, coverage);
        }
    }
}

fn run_search_session(
    mut worker: QueryWorker<'_>,
    cancelled: &AtomicBool,
    commands: &Receiver<SearchCommand>,
) {
    while !cancelled.load(Ordering::Acquire) {
        let first = match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let (next_query, index_changed) = drain_commands(first, commands);
        worker.handle(next_query, index_changed);
    }
}

fn publish(
    sender: &Sender<SearchEvent>,
    progress: &WalkProgress,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let limit = progress.limit;
    let full_page = limit > 0 && progress.matches.len() == limit;
    let _sent = sender.send(SearchEvent::Results {
        query: progress.query.clone(),
        items: progress
            .matches
            .iter()
            .map(|(_, item)| item)
            .filter(|item| search_path_present(&item.path))
            .cloned()
            .collect(),
        indexing,
        coverage,
        has_more: full_page && limit < MAX_INDEX_ENTRIES,
        candidates: progress.matches.len(),
    });
}

fn search_path_present(path: &Path) -> bool {
    // Preserve dangling symlinks and uncertain paths; only confirmed absence removes a hit.
    path.symlink_metadata().map_or_else(
        |error| error.kind() != std::io::ErrorKind::NotFound,
        |_| true,
    )
}
