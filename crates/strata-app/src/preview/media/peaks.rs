// SPDX-License-Identifier: MIT

//! Background audio waveform peaks (`audio-peaks`) for media previews, matching GTK's
//! `load_peaks` (`src/ui/preview/audio/details.rs`).
//!
//! - Uses `strata_preview::sandbox::media::PeaksSession`, which shares a single background slot
//!   with video storyboards; retries while the slot is busy.
//! - Streams `PeaksEvent::Levels { start, levels }` batches into Rune through `Later`, guarded by
//!   the viewer's monotonic id so stale events from a replaced or closed player are discarded.
//! - Dropping [`PeaksLoad`] cancels the sandbox helper and removes its GLib timer immediately.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

use rune::Tx;
use strata_core::model::FileEntry;
use strata_fs::services::SandboxedMedia;
use strata_preview::{
    media::peaks::BUCKETS,
    sandbox::media::{PeaksEvent, PeaksSession},
};

use super::cache::{Lru, TrackKey};
use crate::{App, slots::Later};

const PEAKS_CACHE: usize = 64;
/// Fast browsing must not start a full waveform decode for every track it passes.
const LOAD_SETTLE: Duration = Duration::from_millis(50);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

thread_local! {
    static PEAKS: RefCell<Lru<TrackKey, Rc<Vec<u8>>>> =
        const { RefCell::new(Lru::new(PEAKS_CACHE)) };
}

/// Cached waveform bucket levels for `entry`, if decoded previously.
pub(super) fn cached(entry: &FileEntry) -> Option<Rc<Vec<u8>>> {
    let key = TrackKey::of(entry);
    PEAKS.with_borrow_mut(|cache| cache.get(&key))
}

/// A batch of `(start_bucket, companded_levels)` runs drained from one poll tick.
pub(super) type Batch = Vec<(u32, Vec<u8>)>;

/// Where drained peak batches go: the viewer id and delivery callback on `Later`.
#[derive(Clone)]
pub(super) struct Inlet {
    pub(super) later: Later<App>,
    pub(super) viewer: u64,
    pub(super) deliver: fn(&mut App, u64, Batch, &mut Tx) -> bool,
}

#[cfg(test)]
pub(in crate::preview) struct MockPeaks {
    receiver: std::sync::mpsc::Receiver<PeaksEvent>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl MockPeaks {
    pub(in crate::preview) fn new(
        receiver: std::sync::mpsc::Receiver<PeaksEvent>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            receiver,
            cancelled,
        }
    }

    fn receive(&self) -> Option<PeaksEvent> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(PeaksEvent::Failed),
        }
    }
}

#[cfg(test)]
impl Drop for MockPeaks {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
pub(in crate::preview) type Start = fn(SandboxedMedia) -> Option<MockPeaks>;

#[cfg(test)]
thread_local! {
    /// Test hook replacing `PeaksSession::start` so unit tests never require host `ffmpeg`.
    pub(in crate::preview) static START: std::cell::Cell<Option<Start>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(in crate::preview) fn clear_cache() {
    PEAKS.with_borrow_mut(Lru::clear);
}

enum Source {
    Sandbox(PeaksSession),
    #[cfg(test)]
    Mock(MockPeaks),
}

impl Source {
    fn receive(&self) -> Option<PeaksEvent> {
        match self {
            Self::Sandbox(session) => session.receive(),
            #[cfg(test)]
            Self::Mock(mock) => mock.receive(),
        }
    }
}

enum StartResult {
    Started(Source),
    Busy,
    #[cfg(test)]
    Skip,
}

fn start_source(source: SandboxedMedia) -> StartResult {
    #[cfg(test)]
    {
        if let Some(start) = START.get() {
            return match start(source) {
                Some(mock) => StartResult::Started(Source::Mock(mock)),
                None => StartResult::Busy,
            };
        }
        if super::stream::START.get().is_some() {
            return StartResult::Skip;
        }
    }
    match PeaksSession::start(source) {
        Some(session) => StartResult::Started(Source::Sandbox(session)),
        None => StartResult::Busy,
    }
}

struct LoadState {
    session: RefCell<Option<Source>>,
    timer: RefCell<Option<glib::SourceId>>,
}

/// Active background `audio-peaks` decode; dropping it cancels the helper and timer immediately.
pub(super) struct PeaksLoad {
    state: Rc<LoadState>,
}

impl Drop for PeaksLoad {
    fn drop(&mut self) {
        self.state.session.borrow_mut().take();
        if let Some(timer) = self.state.timer.borrow_mut().take() {
            timer.remove();
        }
    }
}

struct PollContext {
    key: TrackKey,
    source: SandboxedMedia,
    inlet: Inlet,
    accumulated: Vec<u8>,
}

/// Starts a background `audio-peaks` decode for `entry` and streams batches into `inlet`.
pub(super) fn load(entry: &FileEntry, source: SandboxedMedia, inlet: Inlet) -> PeaksLoad {
    let state = Rc::new(LoadState {
        session: RefCell::new(None),
        timer: RefCell::new(None),
    });
    let weak = Rc::downgrade(&state);
    let context = PollContext {
        key: TrackKey::of(entry),
        source,
        inlet,
        accumulated: Vec::with_capacity(BUCKETS as usize),
    };
    let timer = glib::timeout_add_local_once(LOAD_SETTLE, move || begin(&weak, context));
    state.timer.replace(Some(timer));
    PeaksLoad { state }
}

fn begin(weak: &Weak<LoadState>, mut context: PollContext) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    state.timer.borrow_mut().take();
    match start_source(context.source.clone()) {
        StartResult::Started(session) => {
            state.session.replace(Some(session));
        }
        StartResult::Busy => {}
        #[cfg(test)]
        StartResult::Skip => return,
    }
    let poll_weak = weak.clone();
    let timer = glib::timeout_add_local(POLL_INTERVAL, move || poll(&poll_weak, &mut context));
    state.timer.replace(Some(timer));
}

fn poll(weak: &Weak<LoadState>, context: &mut PollContext) -> glib::ControlFlow {
    let Some(state) = weak.upgrade() else {
        return glib::ControlFlow::Break;
    };
    if state.session.borrow().is_none() {
        return retry_start(&state, &context.source);
    }
    let (batch, outcome) = drain(&state, &mut context.accumulated);
    if !batch.is_empty() {
        let (viewer, deliver) = (context.inlet.viewer, context.inlet.deliver);
        context
            .inlet
            .later
            .then(move |app: &mut App, tx: &mut Tx| deliver(app, viewer, batch, tx));
    }
    match outcome {
        DrainOutcome::Pending => glib::ControlFlow::Continue,
        DrainOutcome::Finished => {
            if !context.accumulated.is_empty() {
                let cached = Rc::new(std::mem::take(&mut context.accumulated));
                let key = context.key.clone();
                PEAKS.with_borrow_mut(|cache| cache.insert(key, cached));
            }
            stop(&state)
        }
        DrainOutcome::Failed => stop(&state),
    }
}

fn retry_start(state: &LoadState, source: &SandboxedMedia) -> glib::ControlFlow {
    match start_source(source.clone()) {
        StartResult::Started(session) => {
            state.session.replace(Some(session));
            glib::ControlFlow::Continue
        }
        StartResult::Busy => glib::ControlFlow::Continue,
        #[cfg(test)]
        StartResult::Skip => stop(state),
    }
}

enum DrainOutcome {
    Pending,
    Finished,
    Failed,
}

fn drain(state: &LoadState, accumulated: &mut Vec<u8>) -> (Batch, DrainOutcome) {
    let mut batch = Vec::new();
    let session = state.session.borrow();
    let Some(session) = session.as_ref() else {
        return (batch, DrainOutcome::Failed);
    };
    while let Some(event) = session.receive() {
        match event {
            PeaksEvent::Levels { start, levels } => {
                record_levels(accumulated, start, &levels);
                batch.push((start, levels));
            }
            PeaksEvent::Finished => return (batch, DrainOutcome::Finished),
            PeaksEvent::Failed => return (batch, DrainOutcome::Failed),
        }
    }
    (batch, DrainOutcome::Pending)
}

fn record_levels(accumulated: &mut Vec<u8>, start: u32, levels: &[u8]) {
    let start = start as usize;
    let end = start.saturating_add(levels.len()).min(BUCKETS as usize);
    if start >= end {
        return;
    }
    if accumulated.len() < end {
        accumulated.resize(end, 0);
    }
    accumulated[start..end].copy_from_slice(&levels[..end - start]);
}

fn stop(state: &LoadState) -> glib::ControlFlow {
    state.session.borrow_mut().take();
    state.timer.borrow_mut().take();
    glib::ControlFlow::Break
}
