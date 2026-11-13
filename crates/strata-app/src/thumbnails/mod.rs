// SPDX-License-Identifier: MIT

//! Thumbnails for the views: which entries get one, and the render pool that makes them.
//! Shared code: the Icons grid is its first user, and List's icon column and Columns' rows
//! will read the same window's store.
//!
//! - **The store** of decoded images is strata-components' `ThumbnailStore` (C25), one per
//!   window, in the shell (`shell::thumbnails`): every view reads the same one. Each view
//!   that draws thumbnails owns a viewer in it.
//! - **The render pool** is per process, as GTK's is: its queue and its counts are
//!   interface-thread state of this module, as `RENDER_QUEUE`, `RENDER_RUNNING`,
//!   `HEAVY_RUNNING` and `MODEL_RUNNING` are in GTK's `src/ui/thumbnail/jobs.rs`. A view
//!   pops its store's requests while the pool has room ([`room`]) and [`submit`]s each.
//! - **A job** reads the shared freedesktop cache first (`strata_fs::adapters::thumbnail_cache`)
//!   and decodes a hit to the request's step; a miss waits for a render slot, renders in
//!   `strata-preview`'s sandbox, stores what it rendered in the cache and decodes it. Both
//!   stages run on blocking threads (`gio::spawn_blocking`); each result comes back on the
//!   interface thread, in a future on GLib's default context, and the view's callback queues
//!   it with the part's `later`, as the shell's rules for a worker thread's result say.
//! - **Limits**, as GTK's: [`READERS`] cache reads at once, which never wait for a render
//!   slot; at most `sandbox::browser::worker_limit()` renders at once, one of them kept from
//!   the heavy kinds ([`Kind::heavy`]), and one STL render at a time; and no more than
//!   [`QUEUED`] jobs held besides the renders running.

use std::{
    cell::RefCell,
    collections::VecDeque,
    hash::{BuildHasher, Hash, Hasher, RandomState},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::UNIX_EPOCH,
};

use strata_components::{contract::ThumbKey, thumbnails::Outcome};
use strata_core::model::FileEntry;
use strata_fs::adapters::thumbnail_cache;
use strata_preview::sandbox::{Cancellation, browser};

mod kind;
mod viewer;

use kind::Kind;

use crate::listing::known;
pub(crate) use viewer::Viewer;

/// The key of `entry`'s thumbnail: its path, modification time and size, so a change to the
/// file is a new key. None for an entry that gets no thumbnail: not a local file, or a kind
/// no renderer takes.
pub(crate) fn key_of(entry: &FileEntry) -> Option<ThumbKey> {
    if entry.is_directory() {
        return None;
    }
    let path = entry.location.native_path()?;
    kind::of(path)?;
    let mut hasher = keys().build_hasher();
    path.hash(&mut hasher);
    known(&entry.modified_unix_seconds).hash(&mut hasher);
    known(&entry.size).hash(&mut hasher);
    Some(ThumbKey::new(hasher.finish()))
}

/// The hash keys of this process: random, so no folder of names can be made to collide.
/// Nothing keeps a thumbnail key past the process; the disk cache is keyed by path.
fn keys() -> &'static RandomState {
    static KEYS: OnceLock<RandomState> = OnceLock::new();
    KEYS.get_or_init(RandomState::new)
}

/// One thumbnail to make.
pub(crate) struct Job {
    pub(crate) path: PathBuf,
    /// The long edge, in pixels, the result may have at most.
    pub(crate) edge: u32,
    pub(crate) cancellation: Cancellation,
    /// Hears the result, on the interface thread.
    pub(crate) done: Box<dyn FnOnce(Outcome)>,
}

/// Cache reads running at once: GTK's `MAX_CACHE_READERS`.
const READERS: usize = 4;
/// Jobs the pool holds besides the renders running: GTK's `MAX_QUEUED_THUMBNAILS`.
const QUEUED: usize = 64;

/// A job the pool holds, with what it renders and, once the cache missed, the file's
/// modification time the render stores under.
struct Queued {
    job: Job,
    kind: Kind,
    modified: Option<i64>,
}

/// What the pool starts next.
enum Start {
    Read(Queued),
    Render(Queued),
}

#[derive(Default)]
struct Pool {
    reads: VecDeque<Queued>,
    reading: usize,
    renders: VecDeque<Queued>,
    running: usize,
    heavy: usize,
    model: bool,
}

thread_local! {
    static POOL: RefCell<Pool> = RefCell::new(Pool::default());
}

impl Pool {
    /// The next job the limits let start: a cache read whenever a reader is free, whatever
    /// the renders hold, else a render.
    fn next(&mut self, limit: usize) -> Option<Start> {
        self.next_read()
            .map(Start::Read)
            .or_else(|| self.next_render(limit).map(Start::Render))
    }

    fn next_read(&mut self) -> Option<Queued> {
        if self.reading >= READERS {
            return None;
        }
        self.reads
            .retain(|queued| !queued.job.cancellation.is_cancelled());
        let queued = self.reads.pop_front()?;
        self.reading += 1;
        Some(queued)
    }

    /// A read finished; one that missed waits for a render.
    fn read(&mut self, missed: Option<Queued>) {
        self.reading = self.reading.saturating_sub(1);
        self.renders.extend(missed);
    }

    /// The next waiting render the limits let start, as GTK's `next_render_job` picks it.
    fn next_render(&mut self, limit: usize) -> Option<Queued> {
        if self.running >= limit {
            return None;
        }
        self.renders
            .retain(|queued| !queued.job.cancellation.is_cancelled());
        let heavy_room = self.heavy < limit.saturating_sub(1).max(1);
        let index = self.renders.iter().position(|queued| {
            (queued.kind != Kind::RenderedModel || !self.model)
                && (!queued.kind.heavy() || heavy_room)
        })?;
        let queued = self.renders.remove(index)?;
        self.count(queued.kind, true);
        Some(queued)
    }

    fn count(&mut self, kind: Kind, starting: bool) {
        let change = |count: &mut usize| match starting {
            true => *count += 1,
            false => *count = count.saturating_sub(1),
        };
        change(&mut self.running);
        if kind.heavy() {
            change(&mut self.heavy);
        }
        if kind == Kind::RenderedModel {
            self.model = starting;
        }
    }

    /// How many more jobs the pool takes now: one for each free reader, while it holds
    /// fewer than [`QUEUED`] besides the renders the limit lets run.
    fn room(&self, limit: usize) -> usize {
        let held = self.reads.len() + self.reading + self.renders.len() + self.running;
        READERS
            .saturating_sub(self.reading + self.reads.len())
            .min((QUEUED + limit).saturating_sub(held))
    }
}

/// How many more jobs the pool would take now: a view pops no more requests than this.
pub(crate) fn room() -> usize {
    let limit = browser::worker_limit();
    POOL.with_borrow(|pool| pool.room(limit))
}

/// Queues `job`, and starts what the limits allow. A file no renderer takes fails at once.
pub(crate) fn submit(job: Job) {
    let Some(kind) = kind::of(&job.path) else {
        (job.done)(Outcome::Failed);
        return;
    };
    let queued = Queued {
        job,
        kind,
        modified: None,
    };
    POOL.with_borrow_mut(|pool| pool.reads.push_back(queued));
    start();
}

fn start() {
    let limit = browser::worker_limit();
    while let Some(next) = POOL.with_borrow_mut(|pool| pool.next(limit)) {
        let context = glib::MainContext::default();
        match next {
            Start::Read(queued) => context.spawn_local(read(queued)),
            Start::Render(queued) => context.spawn_local(render(queued)),
        };
    }
}

/// What a cache read found.
enum Looked {
    /// A thumbnail, decoded, or the job's end without one.
    Done(Outcome),
    /// No thumbnail in the cache: the file's modification time, if it has one.
    Missed(Option<i64>),
}

async fn read(mut queued: Queued) {
    let path = queued.job.path.clone();
    let edge = queued.job.edge;
    let cancellation = queued.job.cancellation.clone();
    let looked = gio::spawn_blocking(move || look_up(&path, edge, &cancellation))
        .await
        .unwrap_or(Looked::Done(Outcome::Failed));
    match looked {
        Looked::Done(outcome) => {
            POOL.with_borrow_mut(|pool| pool.read(None));
            (queued.job.done)(outcome);
        }
        Looked::Missed(modified) => {
            queued.modified = modified;
            POOL.with_borrow_mut(|pool| pool.read(Some(queued)));
        }
    }
    start();
}

async fn render(queued: Queued) {
    let Queued {
        job,
        kind,
        modified,
    } = queued;
    let Job {
        path,
        edge,
        cancellation,
        done,
    } = job;
    let outcome = gio::spawn_blocking(move || {
        if cancellation.is_cancelled() {
            return Outcome::Stopped;
        }
        let rendered = kind::render(&path, kind, &cancellation).map(|rendered| rendered.png);
        if let (Ok(png), Some(mtime)) = (&rendered, modified) {
            thumbnail_cache::store(&path, mtime, png);
        }
        made(rendered, edge)
    })
    .await
    .unwrap_or(Outcome::Failed);
    POOL.with_borrow_mut(|pool| pool.count(kind, false));
    done(outcome);
    start();
}

/// `path`'s thumbnail from the shared cache, at most `edge` pixels on its long side.
fn look_up(path: &Path, edge: u32, cancellation: &Cancellation) -> Looked {
    if cancellation.is_cancelled() {
        return Looked::Done(Outcome::Stopped);
    }
    let modified = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_secs()).ok());
    match modified.and_then(|mtime| thumbnail_cache::lookup(path, mtime)) {
        Some(png) => {
            tracing::debug!("browser thumbnail disk hit");
            Looked::Done(decode(&png, edge))
        }
        None => Looked::Missed(modified),
    }
}

/// What a render gives the view: its PNG decoded, or a failure.
fn made(rendered: Result<Vec<u8>, String>, edge: u32) -> Outcome {
    match rendered {
        Ok(png) => decode(&png, edge),
        Err(error) => {
            tracing::debug!(%error, "no thumbnail");
            Outcome::Failed
        }
    }
}

/// The PNG's pixels, scaled down so the long edge is at most `edge`.
fn decode(png: &[u8], edge: u32) -> Outcome {
    let Ok(image) = image::load_from_memory_with_format(png, image::ImageFormat::Png) else {
        return Outcome::Failed;
    };
    let edge = edge.max(1);
    let image = if image.width() > edge || image.height() > edge {
        image.resize(edge, edge, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Outcome::Ready {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

#[cfg(test)]
mod tests;
