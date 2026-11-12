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
//! - **A job** looks in the shared freedesktop cache first
//!   (`strata_fs::adapters::thumbnail_cache`), renders in `strata-preview`'s sandbox on a
//!   miss and stores what it rendered there, then decodes the PNG to the request's step. It
//!   runs on a blocking thread (`gio::spawn_blocking`); its result comes back on the interface
//!   thread, in a future on GLib's default context, and the view's callback queues it with
//!   the part's `later`, as the shell's rules for a worker thread's result say.
//! - **Limits**, as GTK's: at most `sandbox::browser::worker_limit()` jobs at once, one of
//!   them kept from the heavy kinds ([`Kind::heavy`]), and one STL render at a time.

use std::{
    cell::RefCell,
    collections::VecDeque,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    time::UNIX_EPOCH,
};

use strata_components::{contract::ThumbKey, thumbnails::Outcome};
use strata_core::model::{FileEntry, MetadataValue};
use strata_fs::adapters::thumbnail_cache;
use strata_preview::sandbox::{Cancellation, browser};

mod kind;
mod viewer;

use kind::Kind;
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
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    known(&entry.modified_unix_seconds).hash(&mut hasher);
    known(&entry.size).hash(&mut hasher);
    Some(ThumbKey::new(hasher.finish()))
}

fn known<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
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

/// A job the pool holds, with what it renders.
struct Queued {
    job: Job,
    kind: Kind,
}

#[derive(Default)]
struct Pool {
    waiting: VecDeque<Queued>,
    running: usize,
    heavy: usize,
    model: bool,
}

thread_local! {
    static POOL: RefCell<Pool> = RefCell::new(Pool::default());
}

impl Pool {
    /// The next waiting job the limits let start, as GTK's `next_render_job` picks it.
    fn next(&mut self, limit: usize) -> Option<Queued> {
        if self.running >= limit {
            return None;
        }
        self.waiting
            .retain(|queued| !queued.job.cancellation.is_cancelled());
        let heavy_room = self.heavy < limit.saturating_sub(1).max(1);
        let index = self.waiting.iter().position(|queued| {
            (queued.kind != Kind::RenderedModel || !self.model)
                && (!queued.kind.heavy() || heavy_room)
        })?;
        let queued = self.waiting.remove(index)?;
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
}

/// How many more jobs the pool would take now: a view pops no more requests than this.
pub(crate) fn room() -> usize {
    let limit = browser::worker_limit();
    POOL.with_borrow(|pool| limit.saturating_sub(pool.running + pool.waiting.len()))
}

/// Queues `job`, and starts what the limits allow. A file no renderer takes fails at once.
pub(crate) fn submit(job: Job) {
    let Some(kind) = kind::of(&job.path) else {
        (job.done)(Outcome::Failed);
        return;
    };
    POOL.with_borrow_mut(|pool| pool.waiting.push_back(Queued { job, kind }));
    start();
}

fn start() {
    let limit = browser::worker_limit();
    while let Some(queued) = POOL.with_borrow_mut(|pool| pool.next(limit)) {
        glib::MainContext::default().spawn_local(run(queued));
    }
}

async fn run(queued: Queued) {
    let Queued { job, kind } = queued;
    let Job {
        path,
        edge,
        cancellation,
        done,
    } = job;
    let outcome = gio::spawn_blocking(move || make(&path, kind, edge, &cancellation))
        .await
        .unwrap_or(Outcome::Failed);
    POOL.with_borrow_mut(|pool| pool.count(kind, false));
    done(outcome);
    start();
}

/// The thumbnail of `path`, at most `edge` pixels on its long side: from the shared cache,
/// or rendered in the sandbox and stored there.
fn make(path: &std::path::Path, kind: Kind, edge: u32, cancellation: &Cancellation) -> Outcome {
    if cancellation.is_cancelled() {
        return Outcome::Stopped;
    }
    let modified = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_secs()).ok());
    let cached = modified.and_then(|mtime| thumbnail_cache::lookup(path, mtime));
    let png = match cached {
        Some(png) => png,
        None => match kind::render(path, kind, cancellation) {
            Ok(rendered) => {
                if let Some(mtime) = modified {
                    thumbnail_cache::store(path, mtime, &rendered.png);
                }
                rendered.png
            }
            Err(error) => {
                tracing::debug!(%error, "no thumbnail");
                return Outcome::Failed;
            }
        },
    };
    decode(&png, edge)
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
