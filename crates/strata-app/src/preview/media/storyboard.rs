// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

use rune::{Pixels, Tx};
use strata_components::media_player::{Sheet as ComponentSheet, Storyboard};
use strata_core::model::FileEntry;
use strata_fs::services::SandboxedMedia;
use strata_preview::{
    media::storyboard::Sheet,
    sandbox::media::{StoryboardEvent, StoryboardSession},
};

use super::cache::{Lru, TrackKey};
use crate::{App, slots::Later};

pub(super) const MIN_DURATION_US: u64 = strata_preview::media::storyboard::MIN_DURATION_US;
const CELL_EDGE: u32 = 128;
const STORYBOARD_CACHE: usize = 8;
const LOAD_SETTLE: Duration = Duration::from_millis(50);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Clone)]
struct Cached {
    board: Storyboard,
    complete: bool,
}

thread_local! {
    static STORYBOARDS: RefCell<Lru<TrackKey, Cached>> =
        const { RefCell::new(Lru::new(STORYBOARD_CACHE)) };
}

/// Cached storyboard for `entry`, paired with whether all cells finished decoding.
pub(super) fn cached(entry: &FileEntry) -> Option<(Storyboard, bool)> {
    let key = TrackKey::of(entry);
    STORYBOARDS
        .with_borrow_mut(|cache| cache.get(&key))
        .map(|cached| (cached.board, cached.complete))
}

/// Progress reported to the video viewer on `Later`.
pub(super) enum Update {
    Board(Storyboard),
    Done,
}

/// Where storyboard updates go: the viewer id and delivery callback on `Later`.
#[derive(Clone)]
pub(super) struct Inlet {
    pub(super) later: Later<App>,
    pub(super) viewer: u64,
    pub(super) deliver: fn(&mut App, u64, Update, &mut Tx) -> bool,
}

#[cfg(test)]
pub(in crate::preview) struct MockStoryboard {
    receiver: std::sync::mpsc::Receiver<StoryboardEvent>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl MockStoryboard {
    pub(in crate::preview) fn new(
        receiver: std::sync::mpsc::Receiver<StoryboardEvent>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            receiver,
            cancelled,
        }
    }

    fn receive(&self) -> Option<StoryboardEvent> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(StoryboardEvent::Failed),
        }
    }
}

#[cfg(test)]
impl Drop for MockStoryboard {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
pub(in crate::preview) type Start = fn(SandboxedMedia, u32) -> Option<MockStoryboard>;

#[cfg(test)]
thread_local! {
    /// Test hook replacing `StoryboardSession::start` so unit tests never require host `ffmpeg`.
    pub(in crate::preview) static START: std::cell::Cell<Option<Start>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn clear_caches() {
    STORYBOARDS.with_borrow_mut(Lru::clear);
}

#[cfg(test)]
pub(in crate::preview) fn clear_cache() {
    clear_caches();
}

enum Source {
    Sandbox(StoryboardSession),
    #[cfg(test)]
    Mock(MockStoryboard),
}

impl Source {
    fn receive(&self) -> Option<StoryboardEvent> {
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
            return match start(source, CELL_EDGE) {
                Some(mock) => StartResult::Started(Source::Mock(mock)),
                None => StartResult::Busy,
            };
        }
        if super::stream::START.get().is_some() {
            return StartResult::Skip;
        }
    }
    match StoryboardSession::start(source, CELL_EDGE) {
        Some(session) => StartResult::Started(Source::Sandbox(session)),
        None => StartResult::Busy,
    }
}

struct LoadState {
    session: RefCell<Option<Source>>,
    timer: RefCell<Option<glib::SourceId>>,
}

/// Active background `video-storyboard` decode; dropping it cancels the helper and timer.
pub(super) struct StoryboardLoad {
    state: Rc<LoadState>,
}

impl Drop for StoryboardLoad {
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
    sheet: Option<Sheet>,
    board: Option<Storyboard>,
}

/// Starts a background `video-storyboard` decode for `entry`.
pub(super) fn load(entry: &FileEntry, source: SandboxedMedia, inlet: Inlet) -> StoryboardLoad {
    let state = Rc::new(LoadState {
        session: RefCell::new(None),
        timer: RefCell::new(None),
    });
    let weak = Rc::downgrade(&state);
    let context = PollContext {
        key: TrackKey::of(entry),
        source,
        inlet,
        sheet: None,
        board: None,
    };
    let timer = glib::timeout_add_local_once(LOAD_SETTLE, move || begin(&weak, context));
    state.timer.replace(Some(timer));
    StoryboardLoad { state }
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
        StartResult::Skip => {
            send(&context.inlet, Update::Done);
            return;
        }
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
        return retry_start(&state, context);
    }
    let (dirty, outcome) = drain(&state, context);
    if dirty && let Some(board) = context.board.clone() {
        store_cache(&context.key, &board, false);
        send(&context.inlet, Update::Board(board));
    }
    match outcome {
        DrainOutcome::Pending => glib::ControlFlow::Continue,
        DrainOutcome::Finished => {
            if let Some(board) = context.board.clone() {
                store_cache(&context.key, &board, true);
                if !dirty {
                    send(&context.inlet, Update::Board(board));
                }
            }
            finish(&state, &context.inlet)
        }
        DrainOutcome::Failed => finish(&state, &context.inlet),
    }
}

fn retry_start(state: &LoadState, context: &PollContext) -> glib::ControlFlow {
    match start_source(context.source.clone()) {
        StartResult::Started(session) => {
            state.session.replace(Some(session));
            glib::ControlFlow::Continue
        }
        StartResult::Busy => glib::ControlFlow::Continue,
        #[cfg(test)]
        StartResult::Skip => finish(state, &context.inlet),
    }
}

enum DrainOutcome {
    Pending,
    Finished,
    Failed,
}

fn drain(state: &LoadState, context: &mut PollContext) -> (bool, DrainOutcome) {
    let mut dirty = false;
    let session = state.session.borrow();
    let Some(session) = session.as_ref() else {
        return (false, DrainOutcome::Failed);
    };
    while let Some(event) = session.receive() {
        match event {
            StoryboardEvent::Sheet(sheet) => {
                context.sheet = Some(sheet);
                context.board = Some(adopt(&context.key, sheet));
            }
            StoryboardEvent::Cell { index, pixels } => {
                if apply_cell(context, index, pixels) {
                    dirty = true;
                }
            }
            StoryboardEvent::Finished => return (dirty, DrainOutcome::Finished),
            StoryboardEvent::Failed => return (dirty, DrainOutcome::Failed),
        }
    }
    (dirty, DrainOutcome::Pending)
}

fn apply_cell(context: &mut PollContext, index: u32, pixels: Vec<u8>) -> bool {
    let (Some(sheet), Some(board)) = (context.sheet, context.board.as_mut()) else {
        return false;
    };
    if index >= sheet.count || pixels.len() != sheet.cell_bytes() {
        return false;
    }
    let texture = Pixels::new();
    texture.set(sheet.width, sheet.height, pixels);
    board.set_cell(index, texture)
}

fn adopt(key: &TrackKey, sheet: Sheet) -> Storyboard {
    let target = component_sheet(sheet);
    STORYBOARDS.with_borrow_mut(|cache| {
        if let Some(cached) = cache.get(key).filter(|item| item.board.sheet() == target) {
            return cached.board;
        }
        let fresh = Storyboard::new(target);
        cache.insert(
            key.clone(),
            Cached {
                board: fresh.clone(),
                complete: false,
            },
        );
        fresh
    })
}

fn store_cache(key: &TrackKey, board: &Storyboard, complete: bool) {
    STORYBOARDS.with_borrow_mut(|cache| {
        cache.insert(
            key.clone(),
            Cached {
                board: board.clone(),
                complete,
            },
        );
    });
}

fn component_sheet(sheet: Sheet) -> ComponentSheet {
    ComponentSheet {
        cell: [sheet.width, sheet.height],
        count: sheet.count,
        duration: Duration::from_micros(sheet.duration_us),
    }
}

fn send(inlet: &Inlet, update: Update) {
    let (viewer, deliver) = (inlet.viewer, inlet.deliver);
    inlet
        .later
        .then(move |app: &mut App, tx: &mut Tx| deliver(app, viewer, update, tx));
}

fn finish(state: &LoadState, inlet: &Inlet) -> glib::ControlFlow {
    state.session.borrow_mut().take();
    state.timer.borrow_mut().take();
    send(inlet, Update::Done);
    glib::ControlFlow::Break
}
