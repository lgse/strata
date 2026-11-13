// SPDX-License-Identifier: MIT

//! A view's side of thumbnails: its viewer in the window's store, the requests it has sent
//! to the pool, and the way their results come back. Every view that draws thumbnails keeps
//! one, and says where an entry is in the browser its own way ([`Locate`]).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use rune::Tx;
use strata_components::{
    contract::EntryId,
    thumbnails::{Outcome, Request, ThumbnailStore, Ticket, ViewerId},
};
use strata_core::model::{Location, MetadataValue};
use strata_fs::services::MetadataUpdate;
use strata_preview::sandbox::{Cancellation, metadata::MediaMetadata};

use super::{Job, Made, room, submit};
use crate::{App, slots::Later};

/// What a view's own state and the window's store are, borrowed together.
pub(crate) type Parts = fn(&mut App) -> (&mut Viewer, &mut ThumbnailStore);

/// Where entry `id` of what the view shows is in the browser: its column's depth and its
/// position there.
pub(crate) type Locate = fn(&App, EntryId) -> Option<(usize, usize)>;

pub(crate) struct Viewer {
    /// The view's viewers in the store: one for a single grid or list, one for each open
    /// column in Columns ([`Viewer::fit`]). Every request they make comes back here.
    ids: Vec<ViewerId>,
    /// The requests out, each with the switch that stops its render.
    pub(super) pending: HashMap<Ticket, Cancellation>,
    later: Later<App>,
    locate: Locate,
}

/// A request sent to the pool: what its result is for.
struct Sent {
    ticket: Ticket,
    entry: EntryId,
    path: PathBuf,
}

impl Viewer {
    pub(crate) fn new(later: Later<App>, locate: Locate) -> Self {
        Self {
            ids: Vec::new(),
            pending: HashMap::new(),
            later,
            locate,
        }
    }

    /// The view's viewer in the store, once it has one: the first [`Viewer::pump`] takes it.
    pub(crate) fn id(&self) -> Option<ViewerId> {
        self.ids.first().copied()
    }

    /// The store's viewer for the view's part `index` (Columns' column `index`).
    pub(crate) fn id_at(&self, index: usize) -> Option<ViewerId> {
        self.ids.get(index).copied()
    }

    /// Keeps `count` viewers in the store, at least one: new ones for parts that opened, and
    /// the last ones dropped for parts that closed, so the store no longer protects or asks
    /// for what only they showed. Their requests out come back cancelled at the next pump.
    pub(crate) fn fit(&mut self, store: &mut ThumbnailStore, count: usize) {
        let count = count.max(1);
        while self.ids.len() < count {
            self.ids.push(store.viewer());
        }
        while self.ids.len() > count {
            if let Some(id) = self.ids.pop() {
                store.drop_viewer(id);
            }
        }
    }

    /// Stops what the store no longer wants, and asks for what the pool has room for. An
    /// entry with no local file fails its request. Call it from the view's `poll`.
    pub(crate) fn pump(app: &mut App, parts: Parts, now: f64) {
        let (requests, locate) = {
            let (viewer, store) = parts(app);
            (viewer.take_requests(store, now), viewer.locate)
        };
        for request in requests {
            match file(app, locate, request.entry) {
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
        if self.ids.is_empty() {
            self.ids.push(store.viewer());
        }
        for ticket in store.take_cancelled() {
            if let Some(cancellation) = self.pending.remove(&ticket) {
                cancellation.cancel();
            }
        }
        std::iter::from_fn(|| store.pop(now)).take(room()).collect()
    }

    fn send(&mut self, parts: Parts, request: Request, path: PathBuf) {
        let sent = Sent {
            ticket: request.ticket,
            entry: request.entry,
            path: path.clone(),
        };
        let cancellation = Cancellation::default();
        self.pending.insert(sent.ticket, cancellation.clone());
        let later = self.later.clone();
        submit(Job {
            path,
            edge: request.step.px(),
            cancellation,
            done: Box::new(move |made: Made| {
                later.then(move |app: &mut App, tx: &mut Tx| {
                    let (viewer, store) = parts(app);
                    let locate = viewer.locate;
                    let landed = viewer.finish(store, (sent.ticket, made.outcome), tx.clock());
                    if let (true, Some(metadata)) = (landed, made.metadata) {
                        publish(app, locate, &sent, &metadata);
                    }
                    // The image lands in its handle, and published metadata comes back as
                    // the browser's event: nothing the view reads changed here.
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
        (ticket, outcome): (Ticket, Outcome),
        now: f64,
    ) -> bool {
        if self.pending.remove(&ticket).is_none() {
            return false;
        }
        let _ = store.complete(ticket, outcome, now);
        true
    }
}

/// The local file of entry `id`, through the browser.
fn file(app: &App, locate: Locate, id: EntryId) -> Option<PathBuf> {
    let (depth, position) = locate(app, id)?;
    let entry = app.browser.entry_at(depth, position)?;
    entry.location.native_path().map(Path::to_path_buf)
}

/// Hands the browser what the worker read of the file while it rendered, if the entry is
/// still the file it rendered: GTK's `publish_thumbnail_metadata`
/// (`src/ui/thumbnail/viewport.rs`). The browser announces it as a metadata fill.
fn publish(app: &App, locate: Locate, sent: &Sent, metadata: &MediaMetadata) {
    let Some((depth, position)) = locate(app, sent.entry) else {
        return;
    };
    let Some(entry) = app.browser.entry_at(depth, position) else {
        return;
    };
    if entry.location.native_path() != Some(sent.path.as_path()) {
        return;
    }
    let update = update_of(entry.location, metadata);
    app.browser
        .apply_thumbnail_metadata(depth, position, update);
}

/// The fill a render's metadata makes: the dimensions and the length, rounded to a second;
/// nothing else is known by it.
fn update_of(location: Location, metadata: &MediaMetadata) -> MetadataUpdate {
    MetadataUpdate {
        location,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: metadata
            .dimensions
            .map_or(MetadataValue::Unknown, MetadataValue::Known),
        child_count: MetadataValue::Unknown,
        duration_seconds: metadata.duration.map_or(MetadataValue::Unknown, |seconds| {
            MetadataValue::Known(seconds.round() as u64)
        }),
        listable: MetadataValue::Unknown,
    }
}
