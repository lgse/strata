// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

static STARTED: OnceLock<Instant> = OnceLock::new();
static FIRST_BATCH_RENDERED: AtomicBool = AtomicBool::new(false);
static FIRST_THEMED_FRAME: AtomicBool = AtomicBool::new(false);
static FIRST_VISIBLE_ROW: AtomicBool = AtomicBool::new(false);

static THUMB_ELIGIBLE: AtomicU64 = AtomicU64::new(0);
static THUMB_REQUESTED: AtomicU64 = AtomicU64::new(0);
static THUMB_STARTED: AtomicU64 = AtomicU64::new(0);
static THUMB_COMPLETED: AtomicU64 = AtomicU64::new(0);
static THUMB_APPLIED: AtomicU64 = AtomicU64::new(0);
static THUMB_CANCELLED: AtomicU64 = AtomicU64::new(0);
static THUMB_STALE: AtomicU64 = AtomicU64::new(0);

pub fn initialize() {
    let _started = STARTED.set(Instant::now());
}

pub fn mark_window_presented() {
    if let Some(started) = STARTED.get() {
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "application window presented"
        );
    }
}

/// The field harness parses this line for the cold/warm first-frame metric.
pub fn mark_first_themed_frame() {
    if FIRST_THEMED_FRAME.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Some(started) = STARTED.get() {
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "first themed frame painted"
        );
    }
}

/// Emitted once per process so paired runs can compare identical-work latency.
pub fn mark_first_visible_row(entries: usize) {
    if FIRST_VISIBLE_ROW.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Some(started) = STARTED.get() {
        tracing::info!(
            entries,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "first correct visible row published"
        );
    }
}

/// Callers time their own slice and report it here so the harness can
/// attribute wall time without extra global state.
pub fn record_stage(stage: &str, elapsed_ms: u64) {
    tracing::debug!(stage, elapsed_ms, "directory stage completed");
}

#[expect(dead_code, reason = "frame-clock plumbing lands with Task 11")]
pub fn record_input_latency(elapsed_ms: u64) {
    tracing::debug!(elapsed_ms, "input event to presented frame");
}

pub fn mark_batch_rendered(entries: usize, render_started: Instant) {
    let render_micros = render_started.elapsed().as_micros() as u64;
    tracing::debug!(entries, render_micros, "directory batch rendered");

    if !FIRST_BATCH_RENDERED.swap(true, Ordering::Relaxed)
        && let Some(started) = STARTED.get()
    {
        tracing::info!(
            entries,
            render_micros,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "first directory batch rendered"
        );
        mark_first_visible_row(entries);
    }
}

/// Each transition bumps a process-wide counter the harness scrapes at settle time.
pub fn mark_thumbnail_eligible(count: u64) {
    THUMB_ELIGIBLE.fetch_add(count, Ordering::Relaxed);
    tracing::debug!(
        count,
        total = THUMB_ELIGIBLE.load(Ordering::Relaxed),
        "thumbnails eligible"
    );
}

pub fn mark_thumbnail_requested() {
    THUMB_REQUESTED.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        total = THUMB_REQUESTED.load(Ordering::Relaxed),
        "thumbnail requested"
    );
}

pub fn mark_thumbnail_started() {
    THUMB_STARTED.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        total = THUMB_STARTED.load(Ordering::Relaxed),
        "thumbnail started"
    );
}

pub fn mark_thumbnail_completed() {
    THUMB_COMPLETED.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        total = THUMB_COMPLETED.load(Ordering::Relaxed),
        "thumbnail completed"
    );
}

pub fn mark_thumbnail_applied() {
    THUMB_APPLIED.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        total = THUMB_APPLIED.load(Ordering::Relaxed),
        "thumbnail applied"
    );
}

pub fn mark_thumbnail_cancelled() {
    THUMB_CANCELLED.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        total = THUMB_CANCELLED.load(Ordering::Relaxed),
        "thumbnail cancelled"
    );
}

pub fn mark_thumbnail_stale() {
    THUMB_STALE.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        total = THUMB_STALE.load(Ordering::Relaxed),
        "thumbnail stale"
    );
}

pub fn thumbnail_counts() -> ThumbnailCounts {
    ThumbnailCounts {
        eligible: THUMB_ELIGIBLE.load(Ordering::Relaxed),
        requested: THUMB_REQUESTED.load(Ordering::Relaxed),
        started: THUMB_STARTED.load(Ordering::Relaxed),
        completed: THUMB_COMPLETED.load(Ordering::Relaxed),
        applied: THUMB_APPLIED.load(Ordering::Relaxed),
        cancelled: THUMB_CANCELLED.load(Ordering::Relaxed),
        stale: THUMB_STALE.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThumbnailCounts {
    pub eligible: u64,
    pub requested: u64,
    pub started: u64,
    pub completed: u64,
    pub applied: u64,
    pub cancelled: u64,
    pub stale: u64,
}

#[cfg(test)]
mod tests;
