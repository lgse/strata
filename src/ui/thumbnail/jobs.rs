// SPDX-License-Identifier: MIT

//! Running thumbnail jobs: a cache lookup on disk first, then a sandboxed render
//! within the worker limits, then applying the result to every waiting slot.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use gtk::{gdk, glib};

use super::{
    CacheHit, PendingTarget, SettledPark, THUMBNAIL_CACHE, THUMBNAIL_QUEUE, ThumbnailKey,
    ThumbnailKind, background, camera, enqueue_persist, finish_thumbnail_targets, fire_parks,
    heavy, mark_deferred, render_thumbnail, request_is_live, retry_deferred_thumbnails,
    take_pending_targets, viewport,
};
use crate::{
    sandbox::{Cancellation, browser::Thumbnail, metadata::MediaMetadata},
    services::file_activity,
};

// Cache reads never wait for a decoder permit.
thread_local! {
    pub(super) static RENDER_QUEUE: RefCell<VecDeque<ThumbnailJob>> = const { RefCell::new(VecDeque::new()) };
    static RENDER_RUNNING: Cell<usize> = const { Cell::new(0) };
    static HEAVY_RUNNING: Cell<usize> = const { Cell::new(0) };
    pub(super) static MODEL_RUNNING: Cell<bool> = const { Cell::new(false) };
    static RELEASED_THUMBNAILS: RefCell<Vec<SettledPark>> = const { RefCell::new(Vec::new()) };
    static RELEASES: RefCell<Option<file_activity::Subscription>> =
        const { RefCell::new(None) };
}

pub(super) struct ThumbnailJob {
    pub(super) id: u64,
    pub(super) key: ThumbnailKey,
    pub(super) resolved: ThumbnailKey,
    pub(super) kind: ThumbnailKind,
    pub(super) cancellation: Cancellation,
}

/// A finished render: the thumbnail and whether it was rendered rather than read from disk.
type RenderResult = Result<(Thumbnail, bool), String>;

pub(super) fn start_thumbnail_jobs() {
    while let Some(key) = THUMBNAIL_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        if queue.running < super::MAX_CACHE_READERS {
            viewport::prioritize_queue(&mut queue.queued);
        }
        queue.begin_next()
    }) {
        let job = super::PENDING_THUMBNAILS.with(|pending| {
            pending.borrow().get(&key).map(|pending| ThumbnailJob {
                id: pending.id,
                resolved: key.clone(),
                key,
                kind: pending.kind,
                cancellation: pending.cancellation.clone(),
            })
        });
        let Some(job) = job else {
            THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().finish());
            continue;
        };
        crate::metrics::mark_thumbnail_started();
        glib::MainContext::default().spawn_local(run_thumbnail_job(job));
    }
}

async fn run_thumbnail_job(mut job: ThumbnailJob) {
    let cached = read_disk_cache(&mut job).await;
    let memory = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().get(&job.resolved));
    if let Some(hit) = memory {
        finish_from_memory(&job, hit);
    } else if let Some(png) = cached {
        tracing::debug!("browser thumbnail disk hit");
        finish_thumbnail_job(
            job,
            Ok((
                Thumbnail {
                    png,
                    metadata: None,
                },
                false,
            )),
        )
        .await;
    } else if !job.cancellation.is_cancelled() {
        RENDER_QUEUE.with(|queue| queue.borrow_mut().push_back(job));
    }
    THUMBNAIL_QUEUE.with(|queue| queue.borrow_mut().finish());
    continue_pipeline();
}

/// Resolves the job's key against the file's current metadata and reads its
/// cached PNG, if any. Camera files have no disk cache entry.
async fn read_disk_cache(job: &mut ThumbnailJob) -> Option<Vec<u8>> {
    let mut key = job.key.clone();
    if job.kind == ThumbnailKind::Camera {
        return None;
    }
    let lookup = background::cache(move || {
        use std::os::unix::fs::MetadataExt;
        if let Ok(metadata) = std::fs::metadata(&key.path) {
            key.modified = Some(metadata.mtime());
            key.file_size = Some(metadata.len());
        }
        let png = key
            .modified
            .and_then(|mtime| super::super::thumbnail_cache::lookup(&key.path, mtime));
        (key, png)
    })
    .await;
    let (key, png) = lookup.ok()?;
    job.resolved = key;
    png
}

fn finish_from_memory(job: &ThumbnailJob, hit: CacheHit) {
    let Some(targets) = take_pending_targets(&job.key, job.id) else {
        return;
    };
    let texture = match hit {
        CacheHit::Ready(texture) => Some(texture),
        CacheHit::Failed => None,
    };
    finish_thumbnail_targets(targets, texture.as_ref(), &job.key.path);
}

/// Starts whatever a finished job made room for.
fn continue_pipeline() {
    start_render_jobs();
    start_thumbnail_jobs();
    retry_deferred_thumbnails();
}

pub(in crate::ui) fn set_worker_limit(workers: usize) {
    crate::sandbox::browser::set_worker_limit(workers);
    start_render_jobs();
}

pub(super) fn start_render_jobs() {
    let limit = crate::sandbox::browser::worker_limit();
    while RENDER_RUNNING.with(Cell::get) < limit {
        let job = next_render_job(limit);
        let Some(job) = job else {
            break;
        };
        if viewport::key_priority(&job.key).0 >= 2 {
            defer_job(&job);
            continue;
        }
        RENDER_RUNNING.with(|running| running.set(running.get() + 1));
        if heavy(job.kind) {
            HEAVY_RUNNING.with(|running| running.set(running.get() + 1));
        }
        if job.kind == ThumbnailKind::RenderedModel {
            MODEL_RUNNING.with(|running| running.set(true));
        }
        glib::MainContext::default().spawn_local(run_render_job(job));
    }
}

fn defer_job(job: &ThumbnailJob) {
    let Some(targets) = take_pending_targets(&job.key, job.id) else {
        return;
    };
    for target in targets {
        mark_deferred(job.key.clone(), job.kind, target.image_id, target.request);
    }
}

pub(super) fn next_render_job(limit: usize) -> Option<ThumbnailJob> {
    RENDER_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.retain(|job| !job.cancellation.is_cancelled());
        queue
            .make_contiguous()
            .sort_by_cached_key(|job| viewport::key_priority(&job.key));
        let index = queue.iter().position(|job| {
            (job.kind != ThumbnailKind::RenderedModel || !MODEL_RUNNING.with(Cell::get))
                && (!heavy(job.kind)
                    || HEAVY_RUNNING.with(Cell::get) < limit.saturating_sub(1).max(1))
        })?;
        queue.remove(index)
    })
}

async fn run_render_job(job: ThumbnailJob) {
    let preview_turn = (job.kind == ThumbnailKind::Camera)
        .then(|| crate::services::camera_preview::begin(&job.key.path.to_string_lossy()));
    let result = render(&job).await;
    let was_heavy = heavy(job.kind);
    let was_model = job.kind == ThumbnailKind::RenderedModel;
    finish_thumbnail_job(job, result).await;
    RENDER_RUNNING.with(|running| running.set(running.get().saturating_sub(1)));
    if was_heavy {
        HEAVY_RUNNING.with(|running| running.set(running.get().saturating_sub(1)));
    }
    if was_model {
        MODEL_RUNNING.with(|running| running.set(false));
    }
    drop(preview_turn);
    continue_pipeline();
}

async fn render(job: &ThumbnailJob) -> RenderResult {
    if job.kind == ThumbnailKind::Camera {
        return camera::render(&job.key.path, &job.cancellation)
            .await
            .map(|png| {
                (
                    Thumbnail {
                        png,
                        metadata: None,
                    },
                    false,
                )
            });
    }
    let path = job.key.path.clone();
    let kind = job.kind;
    let cancellation = job.cancellation.clone();
    background::render(move || render_thumbnail(&path, kind, &cancellation))
        .await
        .map_err(|_| "Thumbnail worker failed".to_owned())
        .and_then(|result| result)
        .map(|png| (png, true))
}

pub(super) async fn finish_thumbnail_job(job: ThumbnailJob, result: RenderResult) {
    let targets = take_pending_targets(&job.key, job.id);
    let requested = (job.key, job.kind);
    let key = job.resolved;
    if let Some(targets) = targets {
        match result {
            Ok((thumbnail, rendered)) => finish_rendered(key, targets, thumbnail, rendered).await,
            Err(message) if message == file_activity::HELD_FOR_OPERATION => {
                crate::metrics::mark_thumbnail_cancelled();
                admit_after_release(requested, targets);
            }
            Err(_) => {
                crate::metrics::mark_thumbnail_cancelled();
                let path = key.path.clone();
                THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert_failure(key));
                finish_thumbnail_targets(targets, None, &path);
            }
        }
    }
    let counts = crate::metrics::thumbnail_counts();
    tracing::debug!(?counts, "thumbnail pipeline settled");
}

async fn finish_rendered(
    key: ThumbnailKey,
    targets: Vec<PendingTarget>,
    thumbnail: Thumbnail,
    rendered: bool,
) {
    let path = key.path.clone();
    if let Some(metadata) = thumbnail.metadata {
        publish_metadata(&targets, &path, &metadata);
    }
    let png = thumbnail.png;
    crate::metrics::mark_thumbnail_completed();
    let bytes = glib::Bytes::from_owned(png.clone());
    let texture = background::cache(move || gdk::Texture::from_bytes(&bytes).ok())
        .await
        .ok()
        .flatten();
    if let Some(texture) = texture {
        THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key.clone(), texture.clone()));
        finish_thumbnail_targets(targets, Some(&texture), &path);
    } else {
        finish_thumbnail_targets(targets, None, &path);
    }
    if rendered && let Some(mtime) = key.modified {
        enqueue_persist(key.path.clone(), mtime, png);
    }
}

fn publish_metadata(targets: &[PendingTarget], path: &std::path::Path, metadata: &MediaMetadata) {
    for target in targets {
        if request_is_live(target) {
            viewport::publish_thumbnail_metadata(target.image_id, path, metadata);
        }
    }
}

fn admit_after_release((key, kind): (ThumbnailKey, ThumbnailKind), targets: Vec<PendingTarget>) {
    let parks = targets.into_iter().map(|target| SettledPark {
        key: key.clone(),
        kind,
        target,
    });
    if !file_activity::is_releasing() {
        fire_parks(parks.collect());
        return;
    }
    RELEASED_THUMBNAILS.with_borrow_mut(|released| {
        released.retain(|park| request_is_live(&park.target));
        released.extend(parks);
    });
    RELEASES.with_borrow_mut(|releases| {
        releases.get_or_insert_with(|| {
            file_activity::observe(|notice| {
                if let file_activity::Notice::Lifted(_) = notice {
                    fire_parks(RELEASED_THUMBNAILS.take());
                }
            })
        });
    });
}
