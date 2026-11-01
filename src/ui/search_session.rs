// SPDX-License-Identifier: MIT

//! Worker ownership, independent of GTK collections and browser/chooser policy. The caller owns
//! the session. Polling holds only a weak reference; cancellation/drop removes the source and
//! drops the worker handle/receiver. Delivery runs without session borrows and may restart it.

use crate::{
    adapters::probes::{self, Presence},
    model::Location,
    services::{
        NavigationHistory, RefusedFolders, SearchCoverage, SearchEvent, SearchHandle, SearchItem,
        SearchLimits, index_filter, index_folder_paths, index_paths, index_trees_with_exclusions,
    },
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SearchScope {
    Folder,
    Subfolders,
    FolderTerms,
    Paths,
    Folders,
}

impl SearchScope {
    pub(in crate::ui) fn recursive(self) -> bool {
        matches!(self, Self::Subfolders | Self::Paths | Self::Folders)
    }

    pub(in crate::ui) fn fuzzy(self) -> bool {
        matches!(self, Self::FolderTerms | Self::Paths | Self::Folders)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SearchInput {
    pub(super) root: PathBuf,
    pub(super) show_hidden: bool,
    pub(super) scope: SearchScope,
    pub(super) refused: RefusedFolders,
    pub(super) within: Option<PathBuf>,
}

pub(super) struct SearchBatch {
    pub(super) query: String,
    pub(super) items: Vec<SearchItem>,
    pub(super) indexing: bool,
    pub(super) coverage: SearchCoverage,
    pub(super) has_more: bool,
    pub(super) candidates: usize,
}

type Deliver = Rc<dyn Fn(SearchBatch)>;
struct Worker {
    input: SearchInput,
    limits: SearchLimits,
    handle: SearchHandle,
    receiver: Receiver<SearchEvent>,
}

struct Admission {
    input: SearchInput,
    limits: SearchLimits,
    task: glib::JoinHandle<()>,
}

#[derive(Default)]
struct State {
    worker: RefCell<Option<Worker>>,
    query: RefCell<String>,
    generation: Cell<u64>,
    source: RefCell<Option<glib::SourceId>>,
    deliver: RefCell<Option<Deliver>>,
    awaiting: Cell<bool>,
    pruning: RefCell<Option<gio::Cancellable>>,
    admission: RefCell<Option<Admission>>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(source) = self.source.get_mut().take() {
            source.remove();
        }
        if let Some(pruning) = self.pruning.get_mut().take() {
            pruning.cancel();
        }
        if let Some(admission) = self.admission.get_mut().take() {
            admission.task.abort();
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct SearchSession(Rc<State>);

impl SearchSession {
    pub(super) fn is_active(&self) -> bool {
        self.0.worker.borrow().is_some() || self.0.admission.borrow().is_some()
    }

    pub(super) fn awaiting_results(&self) -> bool {
        self.0.awaiting.get()
    }

    pub(super) fn searches(&self, input: &SearchInput) -> bool {
        self.started_with(|current, _| current == input)
    }

    fn started_with(&self, matches: impl Fn(&SearchInput, SearchLimits) -> bool) -> bool {
        if let Some(worker) = self.0.worker.borrow().as_ref() {
            return matches(&worker.input, worker.limits);
        }
        self.0
            .admission
            .borrow()
            .as_ref()
            .is_some_and(|admission| matches(&admission.input, admission.limits))
    }

    pub(super) fn cancel(&self) {
        self.0
            .generation
            .set(self.0.generation.get().wrapping_add(1));
        if let Some(source) = self.0.source.take() {
            source.remove();
        }
        if let Some(pruning) = self.0.pruning.take() {
            pruning.cancel();
        }
        if let Some(admission) = self.0.admission.take() {
            admission.task.abort();
        }
        self.0.worker.take();
        self.0.deliver.take();
        self.0.awaiting.set(false);
        self.0.query.borrow_mut().clear();
    }

    pub(super) fn prune_missing(
        &self,
        paths: Vec<PathBuf>,
        apply: impl FnOnce(HashSet<PathBuf>) + 'static,
    ) {
        if paths.is_empty() {
            return;
        }
        let cancellable = gio::Cancellable::new();
        if let Some(previous) = self.0.pruning.replace(Some(cancellable.clone())) {
            previous.cancel();
        }
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        glib::MainContext::default().spawn_local(async move {
            let locations: Vec<_> = paths.iter().map(Location::local).collect();
            let found = probes::presence(&locations, &cancellable).await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            if cancellable.is_cancelled() || state.generation.get() != generation {
                return;
            }
            state.pruning.take();
            let gone: HashSet<PathBuf> = paths
                .into_iter()
                .zip(found)
                .filter(|(_, presence)| matches!(presence, Presence::Absent))
                .map(|(path, _)| path)
                .collect();
            if !gone.is_empty() {
                apply(gone);
            }
        });
    }

    pub(super) fn set_show_hidden(&self, show_hidden: bool) {
        let input = self
            .0
            .worker
            .borrow()
            .as_ref()
            .map(|worker| worker.input.clone())
            .or_else(|| {
                self.0
                    .admission
                    .borrow()
                    .as_ref()
                    .map(|admission| admission.input.clone())
            });
        let deliver = self.0.deliver.borrow().clone();
        let (Some(mut input), Some(deliver)) = (input, deliver) else {
            return;
        };
        if input.show_hidden == show_hidden {
            return;
        }
        input.show_hidden = show_hidden;
        let query = self.0.query.borrow().clone();
        self.update(input, &query, false, deliver);
    }

    pub(super) fn expect_query(&self, query: &str) {
        self.0.query.replace(query.trim().to_owned());
    }

    pub(super) fn query(&self, query: &str) {
        self.expect_query(query);
        if let Some(worker) = self.0.worker.borrow().as_ref() {
            self.0.awaiting.set(true);
            worker.handle.query(query.trim());
        }
    }

    pub(super) fn query_candidates(&self, query: &str, count: usize) {
        if *self.0.query.borrow() == query
            && let Some(worker) = self.0.worker.borrow().as_ref()
        {
            worker.handle.query_candidates(query, count);
        }
    }

    /// Only folder pickers accept empty queries; other empty queries cancel.
    pub(super) fn update(&self, input: SearchInput, query: &str, restart: bool, deliver: Deliver) {
        if query.trim().is_empty() && input.scope != SearchScope::Folders {
            self.cancel();
            return;
        }
        let limits = crate::ui::preferences::PreferenceManager::shared().search_limits();
        let replace = restart
            || !self.started_with(|current, current_limits| {
                *current == input && current_limits == limits
            });
        if !replace {
            self.0.deliver.replace(Some(deliver));
            self.query(query);
            return;
        }
        self.cancel();
        self.0.deliver.replace(Some(deliver));
        self.expect_query(query);
        let Some(within) = input.within.clone() else {
            self.start(input, limits, true);
            return;
        };
        self.0.awaiting.set(true);
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        let root = input.root.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let inside = probes::real_folder(&root)
                .await
                .is_ok_and(|real| real.starts_with(&within));
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.generation.get() != generation {
                return;
            }
            if let Some(admission) = state.admission.take() {
                SearchSession(state).start(admission.input, admission.limits, inside);
            }
        });
        self.0.admission.replace(Some(Admission {
            input,
            limits,
            task,
        }));
    }

    fn start(&self, input: SearchInput, limits: SearchLimits, walk: bool) {
        let (handle, receiver) = match input.scope {
            _ if !walk => index_trees_with_exclusions(Vec::new(), false, Vec::new(), limits),
            SearchScope::Folders => index_folder_paths(
                input.root.clone(),
                input.show_hidden,
                NavigationHistory::shared().frecency_within(&input.root),
                input.refused.clone(),
                limits,
            ),
            scope if scope.fuzzy() => index_paths(
                input.root.clone(),
                input.show_hidden,
                scope.recursive(),
                NavigationHistory::shared().frecency_within(&input.root),
                limits,
            ),
            scope => index_filter(
                input.root.clone(),
                input.show_hidden,
                scope.recursive(),
                limits,
            ),
        };
        self.0.worker.replace(Some(Worker {
            input,
            limits,
            handle,
            receiver,
        }));
        let query = self.0.query.borrow().clone();
        self.query(&query);
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        let source = glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let (latest, disconnected) = {
                let worker = state.worker.borrow();
                let Some(worker) = worker.as_ref() else {
                    return glib::ControlFlow::Break;
                };
                let lists_without_query = worker.input.scope == SearchScope::Folders;
                drain(&worker.receiver, &state.query.borrow(), lists_without_query)
            };
            if let Some(batch) = latest {
                state.awaiting.set(false);
                let deliver = state.deliver.borrow().clone();
                if let Some(deliver) = deliver {
                    deliver(batch);
                }
            }
            if state.generation.get() != generation {
                return glib::ControlFlow::Break;
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

fn drain(
    receiver: &Receiver<SearchEvent>,
    current: &str,
    lists_without_query: bool,
) -> (Option<SearchBatch>, bool) {
    let mut latest = None;
    let mut disconnected = false;
    for _ in 0..8 {
        match receiver.try_recv() {
            Ok(SearchEvent::Results {
                query,
                items,
                indexing,
                coverage,
                has_more,
                candidates,
            }) => {
                if (lists_without_query || !query.is_empty()) && query == current {
                    latest = Some(SearchBatch {
                        query,
                        items,
                        indexing,
                        coverage,
                        has_more,
                        candidates,
                    });
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                disconnected = true;
                break;
            }
        }
    }
    (latest, disconnected)
}

#[cfg(test)]
mod tests;
