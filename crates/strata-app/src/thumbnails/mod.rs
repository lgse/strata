// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::VecDeque,
    hash::{BuildHasher, Hasher, RandomState},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::UNIX_EPOCH,
};

use strata_components::{contract::ThumbKey, thumbnails::Outcome};
use strata_core::model::FileEntry;
use strata_fs::adapters::thumbnail_cache;
use strata_preview::sandbox::{
    Cancellation,
    browser::{self, Thumbnail},
    metadata::MediaMetadata,
};

mod kind;
mod viewer;

use kind::Kind;

use crate::listing::known;
pub(crate) use viewer::Viewer;

pub(crate) fn classify(entry: &FileEntry) -> (Option<ThumbKey>, bool) {
    if entry.is_directory() {
        return (None, false);
    }
    let Some(path) = entry.location.native_path() else {
        return (None, false);
    };
    let Some(kind) = kind::of(path) else {
        return (None, false);
    };
    let key = hash_thumb_key(
        path.as_os_str().as_encoded_bytes(),
        known(&entry.modified_unix_seconds),
        known(&entry.size),
    );
    (Some(ThumbKey::new(key)), kind == Kind::RenderedModel)
}

fn thumb_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| RandomState::new().build_hasher().finish())
}

/// Fast word-at-a-time 64-bit hash over raw path bytes plus optional `(mtime, size)`.
fn hash_thumb_key(path_bytes: &[u8], modified: Option<i64>, size: Option<u64>) -> u64 {
    const K: u64 = 0x517cc1b727220a95;
    let mut h = thumb_seed();
    let (chunks, rem) = path_bytes.as_chunks::<8>();
    for &chunk in chunks {
        let word = u64::from_ne_bytes(chunk);
        h = (h.rotate_left(5) ^ word).wrapping_mul(K);
    }
    if !rem.is_empty() {
        let mut tail = [0_u8; 8];
        tail[..rem.len()].copy_from_slice(rem);
        let word = u64::from_ne_bytes(tail);
        h = (h.rotate_left(5) ^ word ^ (rem.len() as u64)).wrapping_mul(K);
    }
    let (m_tag, m_val) = match modified {
        Some(v) => (1_u64, v.cast_unsigned()),
        None => (0_u64, 0_u64),
    };
    let (s_tag, s_val) = match size {
        Some(v) => (1_u64, v),
        None => (0_u64, 0_u64),
    };
    h = (h.rotate_left(5) ^ ((m_tag << 1) | s_tag)).wrapping_mul(K);
    h = (h.rotate_left(5) ^ m_val).wrapping_mul(K);
    h = (h.rotate_left(5) ^ s_val).wrapping_mul(K);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h
}

/// One thumbnail to make.
pub(crate) struct Job {
    pub(crate) path: PathBuf,
    /// The long edge, in pixels, the result may have at most.
    pub(crate) edge: u32,
    pub(crate) cancellation: Cancellation,
    /// Hears the result, on the interface thread.
    pub(crate) done: Box<dyn FnOnce(Made)>,
}

pub(crate) struct Made {
    pub(crate) outcome: Outcome,
    pub(crate) metadata: Option<MediaMetadata>,
}

impl From<Outcome> for Made {
    fn from(outcome: Outcome) -> Self {
        Self {
            outcome,
            metadata: None,
        }
    }
}

const READERS: usize = 4;
const QUEUED: usize = 64;

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
        (job.done)(Outcome::Failed.into());
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
            (queued.job.done)(outcome.into());
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
    let made = gio::spawn_blocking(move || {
        if cancellation.is_cancelled() {
            return Outcome::Stopped.into();
        }
        let rendered = kind::render(&path, kind, &cancellation);
        if let (Ok(rendered), Some(mtime)) = (&rendered, modified) {
            thumbnail_cache::store(&path, mtime, &rendered.png);
        }
        made(rendered, edge)
    })
    .await
    .unwrap_or_else(|_| Outcome::Failed.into());
    POOL.with_borrow_mut(|pool| pool.count(kind, false));
    done(made);
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

fn made(rendered: Result<Thumbnail, String>, edge: u32) -> Made {
    match rendered {
        Ok(rendered) => Made {
            outcome: decode(&rendered.png, edge),
            metadata: rendered.metadata,
        },
        Err(error) => {
            tracing::debug!(%error, "no thumbnail");
            Outcome::Failed.into()
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
