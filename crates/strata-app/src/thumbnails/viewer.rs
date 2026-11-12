// SPDX-License-Identifier: MIT

//! A view's side of thumbnails: its viewer in the window's store, the requests it has sent
//! to the pool, and the way their results come back. Every view that draws thumbnails keeps
//! one, and finds the file of an entry its own way ([`Viewer::pump`]'s `resolve`).

use std::{collections::HashMap, path::PathBuf};

use rune::Tx;
use strata_components::{
    contract::EntryId,
    thumbnails::{Outcome, Request, ThumbnailStore, Ticket, ViewerId},
};
use strata_preview::sandbox::Cancellation;

use super::{Job, room, submit};
use crate::{App, slots::Later};

/// What a view's own state and the window's store are, borrowed together.
pub(crate) type Parts = fn(&mut App) -> (&mut Viewer, &mut ThumbnailStore);

pub(crate) struct Viewer {
    id: Option<ViewerId>,
    /// The requests out, each with the switch that stops its render.
    pub(super) pending: HashMap<Ticket, Cancellation>,
    later: Later<App>,
}

impl Viewer {
    pub(crate) fn new(later: Later<App>) -> Self {
        Self {
            id: None,
            pending: HashMap::new(),
            later,
        }
    }

    /// The view's viewer in the store, once it has one: the first [`Viewer::pump`] takes it.
    pub(crate) fn id(&self) -> Option<ViewerId> {
        self.id
    }

    /// Stops what the store no longer wants, and asks for what the pool has room for:
    /// `resolve` finds an entry's file, or nothing, which fails the request. Call it from the
    /// view's `poll`.
    pub(crate) fn pump(
        app: &mut App,
        parts: Parts,
        resolve: impl Fn(&App, EntryId) -> Option<PathBuf>,
        now: f64,
    ) {
        let requests = {
            let (viewer, store) = parts(app);
            viewer.take_requests(store, now)
        };
        for request in requests {
            match resolve(app, request.entry) {
                Some(path) => {
                    let (viewer, _) = parts(app);
                    viewer.send(parts, request, path);
                }
                None => {
                    let (_, store) = parts(app);
                    let _ = store.complete(request.ticket, Outcome::Failed, now);
                }
            }
        }
    }

    pub(super) fn take_requests(&mut self, store: &mut ThumbnailStore, now: f64) -> Vec<Request> {
        if self.id.is_none() {
            self.id = Some(store.viewer());
        }
        for ticket in store.take_cancelled() {
            if let Some(cancellation) = self.pending.remove(&ticket) {
                cancellation.cancel();
            }
        }
        std::iter::from_fn(|| store.pop(now)).take(room()).collect()
    }

    fn send(&mut self, parts: Parts, request: Request, path: PathBuf) {
        let ticket = request.ticket;
        let cancellation = Cancellation::default();
        self.pending.insert(ticket, cancellation.clone());
        let later = self.later.clone();
        submit(Job {
            path,
            edge: request.step.px(),
            cancellation,
            done: Box::new(move |outcome| {
                later.then(move |app: &mut App, tx: &mut Tx| {
                    let (viewer, store) = parts(app);
                    viewer.finish(store, ticket, outcome, tx.clock());
                    // The image lands in its handle: nothing the view reads changed.
                    false
                });
            }),
        });
    }

    /// Hands a result to the store, unless its request is no longer out: the folder was left
    /// or the view went away since, and the result is dropped without touching the store.
    pub(crate) fn finish(
        &mut self,
        store: &mut ThumbnailStore,
        ticket: Ticket,
        outcome: Outcome,
        now: f64,
    ) -> bool {
        if self.pending.remove(&ticket).is_none() {
            return false;
        }
        let _ = store.complete(ticket, outcome, now);
        true
    }
}
