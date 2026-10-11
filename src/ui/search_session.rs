// SPDX-License-Identifier: MIT

//! Worker ownership, independent of GTK collections and browser/chooser policy. The caller owns
//! the session. Polling holds only a weak reference; cancellation/drop removes the source and
//! drops the worker handle/receiver. Delivery runs without session borrows and may restart it.

use crate::services::{
    NavigationHistory, RefusedFolders, SearchCoverage, SearchEvent, SearchHandle, SearchItem,
    index_filter, index_folder_paths, index_paths,
};
use gtk::glib;
use std::{
    cell::{Cell, RefCell},
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
}

pub(super) struct SearchBatch {
    pub(super) query: String,
    pub(super) items: Vec<SearchItem>,
    pub(super) indexing: bool,
    pub(super) coverage: SearchCoverage,
    pub(super) has_more: bool,
}

type Deliver = Rc<dyn Fn(SearchBatch)>;
struct Worker {
    input: SearchInput,
    handle: SearchHandle,
    receiver: Receiver<SearchEvent>,
}

#[derive(Default)]
struct State {
    worker: RefCell<Option<Worker>>,
    query: RefCell<String>,
    generation: Cell<u64>,
    source: RefCell<Option<glib::SourceId>>,
    deliver: RefCell<Option<Deliver>>,
    awaiting: Cell<bool>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(source) = self.source.get_mut().take() {
            source.remove();
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct SearchSession(Rc<State>);

impl SearchSession {
    pub(super) fn is_active(&self) -> bool {
        self.0.worker.borrow().is_some()
    }

    pub(super) fn awaiting_results(&self) -> bool {
        self.0.awaiting.get()
    }

    pub(super) fn searches(&self, input: &SearchInput) -> bool {
        self.0
            .worker
            .borrow()
            .as_ref()
            .is_some_and(|worker| worker.input == *input)
    }

    pub(super) fn cancel(&self) {
        self.0
            .generation
            .set(self.0.generation.get().wrapping_add(1));
        if let Some(source) = self.0.source.take() {
            source.remove();
        }
        self.0.worker.take();
        self.0.deliver.take();
        self.0.awaiting.set(false);
        self.0.query.borrow_mut().clear();
    }

    pub(super) fn set_show_hidden(&self, show_hidden: bool) {
        let input = self
            .0
            .worker
            .borrow()
            .as_ref()
            .map(|worker| worker.input.clone());
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
        let replace = restart
            || self
                .0
                .worker
                .borrow()
                .as_ref()
                .is_none_or(|worker| worker.input != input);
        if !replace {
            self.0.deliver.replace(Some(deliver));
            self.query(query);
            return;
        }
        self.cancel();
        let (handle, receiver) = match input.scope {
            SearchScope::Folders => index_folder_paths(
                input.root.clone(),
                input.show_hidden,
                NavigationHistory::shared().frecency_within(&input.root),
                input.refused.clone(),
            ),
            scope if scope.fuzzy() => index_paths(
                input.root.clone(),
                input.show_hidden,
                scope.recursive(),
                NavigationHistory::shared().frecency_within(&input.root),
            ),
            scope => index_filter(input.root.clone(), input.show_hidden, scope.recursive()),
        };
        self.0.worker.replace(Some(Worker {
            input,
            handle,
            receiver,
        }));
        self.0.deliver.replace(Some(deliver));
        self.query(query);
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
            }) => {
                if (lists_without_query || !query.is_empty()) && query == current {
                    latest = Some(SearchBatch {
                        query,
                        items,
                        indexing,
                        coverage,
                        has_more,
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
