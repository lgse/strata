// SPDX-License-Identifier: MIT

//! The search behind a filter: GTK's `SearchSession` (`src/ui/search_session.rs`), the
//! index of a folder on strata-fs's worker and its results for the current query.
//!
//! - **One index for a folder and its scope.** A query that keeps the folder, whether hidden
//!   entries show, the subfolders choice and the search limits asks the running index again;
//!   anything else starts a new one, as `SearchSession::update` does.
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

use strata_fs::services::{SearchEvent, SearchHandle, SearchItem, SearchLimits, index_filter};

/// How often GTK reads the worker's channel, and how many events at a time.
const POLL: Duration = Duration::from_millis(16);
const PER_POLL: usize = 8;

/// What a search runs over: the folder, whether hidden entries show, and whether it goes
/// into subfolders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Input {
    pub(crate) root: PathBuf,
    pub(crate) show_hidden: bool,
    pub(crate) recursive: bool,
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
        if query.is_empty() {
            self.cancel();
            return;
        }
        let same = self
            .0
            .worker
            .borrow()
            .as_ref()
            .is_some_and(|worker| worker.input == input && worker.limits == limits);
        if same {
            self.ask(query);
            return;
        }
        self.cancel();
        let (handle, receiver) = index_filter(
            input.root.clone(),
            input.show_hidden,
            input.recursive,
            limits,
        );
        self.0.worker.replace(Some(Worker {
            input,
            limits,
            handle,
        }));
        self.ask(query);
        self.listen(receiver, deliver);
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
            let (latest, disconnected) = drain(&receiver, &state.query.borrow());
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
fn drain(receiver: &Receiver<SearchEvent>, current: &str) -> (Option<Batch>, bool) {
    let mut latest = None;
    for _ in 0..PER_POLL {
        match receiver.try_recv() {
            Ok(event) => {
                if let Some(batch) = batch_for(event, current) {
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
fn batch_for(event: SearchEvent, current: &str) -> Option<Batch> {
    let SearchEvent::Results {
        query,
        items,
        indexing,
        has_more,
        candidates,
        ..
    } = event;
    (!query.is_empty() && query == current).then_some(Batch {
        generation: 0,
        query,
        items,
        incomplete: indexing || has_more,
        has_more,
        candidates,
    })
}

#[cfg(test)]
mod tests;
