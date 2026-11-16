// SPDX-License-Identifier: MIT

//! The search behind a filter and the search palette: GTK's `SearchSession`
//! (`src/ui/search_session.rs`), the index of a folder on strata-fs's worker and its results
//! for the current query, and the palette's index of its roots (`start_search` in
//! `src/ui/search.rs`).
//!
//! - **One index for a scope.** A query that keeps the scope, whether hidden entries show
//!   and the search limits asks the running index again; anything else starts a new one, as
//!   `SearchSession::update` does. The palette starts its index on open, before a query
//!   ([`Session::prepare`]).
//! - **Results on GLib's clock.** GTK reads the worker's channel every 16 ms, eight events
//!   at a time, keeping only the last batch for the query typed now (`drain`); the batch is
//!   queued for the interface through [`Later`], as every service's callback is.
//! - **Cancelling** drops the worker and its timer: a batch already queued names a generation
//!   that has gone and is dropped where it lands.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use strata_fs::services::{
    NavigationHistory, RefusedFolders, SearchCoverage, SearchEvent, SearchHandle, SearchItem,
    SearchLimits, index_filter, index_folder_paths, index_paths, index_trees_with_exclusions,
};

/// How often GTK reads the worker's channel, and how many events at a time.
const POLL: Duration = Duration::from_millis(16);
const PER_POLL: usize = 8;

/// What a search runs over, and whether hidden entries show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Input {
    pub(crate) scope: Scope,
    pub(crate) show_hidden: bool,
}

/// What an index reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// A filter's folder, and whether it goes into subfolders (`index_filter`).
    Folder { root: PathBuf, recursive: bool },
    /// 10xer's fuzzy term/path index (`index_paths`): non-recursive for `f`, recursive for `s`.
    Paths { root: PathBuf, recursive: bool },
    /// 10xer's folder-picker index (`index_folder_paths`), which also lists empty queries.
    Folders {
        root: PathBuf,
        refused: RefusedFolders,
    },
    /// The palette's roots, every subfolder but the excluded ones
    /// (`index_trees_with_exclusions`).
    Trees {
        roots: Vec<PathBuf>,
        exclusions: Vec<String>,
    },
}

impl Scope {
    pub(crate) fn lists_without_query(&self) -> bool {
        matches!(self, Self::Folders { .. })
    }
}

impl Input {
    fn start(&self, limits: SearchLimits) -> (SearchHandle, Receiver<SearchEvent>) {
        match &self.scope {
            Scope::Folder { root, recursive } => {
                index_filter(root.clone(), self.show_hidden, *recursive, limits)
            }
            Scope::Paths { root, recursive } => index_paths(
                root.clone(),
                self.show_hidden,
                *recursive,
                NavigationHistory::shared().frecency_within(root),
                limits,
            ),
            Scope::Folders { root, refused } => index_folder_paths(
                root.clone(),
                self.show_hidden,
                NavigationHistory::shared().frecency_within(root),
                refused.clone(),
                limits,
            ),
            Scope::Trees { roots, exclusions } => index_trees_with_exclusions(
                roots.clone(),
                self.show_hidden,
                exclusions.clone(),
                limits,
            ),
        }
    }
}

/// One batch of results for a query: everything found so far, ranked.
pub(crate) struct Batch {
    pub(crate) generation: u64,
    pub(crate) query: String,
    pub(crate) items: Vec<SearchItem>,
    /// The index is still being read, or the worker holds more candidates than it sent.
    pub(crate) incomplete: bool,
    pub(crate) has_more: bool,
    pub(crate) candidates: usize,
    /// The index is still being read.
    pub(crate) indexing: bool,
    /// What the index left out, by its limits.
    pub(crate) coverage: SearchCoverage,
}

struct Worker {
    input: Input,
    limits: SearchLimits,
    handle: SearchHandle,
}

#[derive(Default)]
struct State {
    worker: RefCell<Option<Worker>>,
    /// The trimmed query whose results are wanted.
    query: RefCell<String>,
    generation: Cell<u64>,
    source: RefCell<Option<glib::SourceId>>,
    /// A query was asked and no batch for it has come.
    awaiting: Cell<bool>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(source) = self.source.get_mut().take() {
            source.remove();
        }
    }
}

/// A filter's search, held by its part. Batches go to `deliver`, on the interface thread.
#[derive(Clone, Default)]
pub(crate) struct Session(Rc<State>);

impl Session {
    /// Whether an index runs.
    pub(crate) fn is_active(&self) -> bool {
        self.0.worker.borrow().is_some()
    }

    /// Whether a query was asked and its first batch has not come.
    pub(crate) fn awaiting(&self) -> bool {
        self.0.awaiting.get()
    }

    /// The generation a batch must carry to be current.
    pub(crate) fn generation(&self) -> u64 {
        self.0.generation.get()
    }

    /// Stops the search: no batch of it is current any more.
    pub(crate) fn cancel(&self) {
        let state = &self.0;
        state.generation.set(state.generation.get().wrapping_add(1));
        if let Some(source) = state.source.take() {
            source.remove();
        }
        state.worker.take();
        state.awaiting.set(false);
        state.query.borrow_mut().clear();
    }

    /// Asks for `query`'s results over `input`, joining the running index when it is the
    /// same search. An empty query cancels.
    pub(crate) fn update(
        &self,
        input: Input,
        query: &str,
        limits: SearchLimits,
        deliver: impl Fn(Batch) + 'static,
    ) {
        let query = query.trim();
        if query.is_empty() && !input.scope.lists_without_query() {
            self.cancel();
            return;
        }
        self.prepare(input, limits, deliver);
        self.ask(query);
    }

    /// Starts the index of `input`, unless it runs already: the palette reads its roots from
    /// the moment it opens, as GTK's `show` starts the search before anything is typed.
    pub(crate) fn prepare(
        &self,
        input: Input,
        limits: SearchLimits,
        deliver: impl Fn(Batch) + 'static,
    ) {
        let same = self
            .0
            .worker
            .borrow()
            .as_ref()
            .is_some_and(|worker| worker.input == input && worker.limits == limits);
        if same {
            return;
        }
        self.cancel();
        let (handle, receiver) = input.start(limits);
        self.0.worker.replace(Some(Worker {
            input,
            limits,
            handle,
        }));
        self.listen(receiver, deliver);
    }

    /// The text the field holds now, before its wait ends: a batch for any other query is
    /// dropped as it arrives (GTK's `expect_query`).
    pub(crate) fn expect(&self, query: &str) {
        self.0.query.replace(query.trim().to_owned());
    }

    /// Asks the worker for more candidates of `query`, when the shown results fell short of
    /// the cap after the interface's own checks (`eligible_results`).
    pub(crate) fn more(&self, query: &str, count: usize) {
        if *self.0.query.borrow() != query {
            return;
        }
        if let Some(worker) = self.0.worker.borrow().as_ref() {
            worker.handle.query_candidates(query, count);
        }
    }

    fn ask(&self, query: &str) {
        self.0.query.replace(query.to_owned());
        if let Some(worker) = self.0.worker.borrow().as_ref() {
            self.0.awaiting.set(true);
            worker.handle.query(query);
        }
    }

    fn listen(&self, receiver: Receiver<SearchEvent>, deliver: impl Fn(Batch) + 'static) {
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        let source = glib::timeout_add_local(POLL, move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let lists_without_query = state
                .worker
                .borrow()
                .as_ref()
                .is_some_and(|w| w.input.scope.lists_without_query());
            let (latest, disconnected) =
                drain(&receiver, &state.query.borrow(), lists_without_query);
            if let Some(mut batch) = latest {
                state.awaiting.set(false);
                batch.generation = generation;
                deliver(batch);
            }
            if disconnected {
                state.source.take();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        self.0.source.replace(Some(source));
    }
}

/// The last batch among the next few events that answers the query typed now, and whether
/// the worker has gone.
fn drain(
    receiver: &Receiver<SearchEvent>,
    current: &str,
    lists_without_query: bool,
) -> (Option<Batch>, bool) {
    let mut latest = None;
    for _ in 0..PER_POLL {
        match receiver.try_recv() {
            Ok(event) => {
                if let Some(batch) = batch_for(event, current, lists_without_query) {
                    latest = Some(batch);
                }
            }
            Err(TryRecvError::Empty) => return (latest, false),
            Err(TryRecvError::Disconnected) => return (latest, true),
        }
    }
    (latest, false)
}

/// `event` as a batch, when it answers `current`.
fn batch_for(event: SearchEvent, current: &str, lists_without_query: bool) -> Option<Batch> {
    let SearchEvent::Results {
        query,
        items,
        indexing,
        coverage,
        has_more,
        candidates,
    } = event;
    ((!query.is_empty() || lists_without_query) && query == current).then_some(Batch {
        generation: 0,
        query,
        items,
        incomplete: indexing || has_more,
        has_more,
        candidates,
        indexing,
        coverage,
    })
}

#[cfg(test)]
mod tests;
