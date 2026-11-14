// SPDX-License-Identifier: MIT

//! The session thread's way into the window's loop. Rune runs the window on the main thread
//! and owns the application state there, so the session reads it through
//! `AppHandle::update_if` and waits for the answer, and reads the tree from the frames the
//! window draws (`App::on_frame_drawn`), which [`Frames`] keeps.

use std::{
    sync::{Arc, Condvar, Mutex, PoisonError, mpsc},
    time::{Duration, Instant},
};

use rune::AppHandle;
use serde_json::Value;

use super::super::{stage, tree::Drawn};
use crate::App;

/// How long a question to the window's loop may take: one turn of it, at most a few frames
/// when it is busy.
const ANSWER_LIMIT: Duration = Duration::from_secs(10);

/// One frame the window drew, as the session reads it.
pub(super) struct Frame {
    pub(super) serial: u64,
    pub(super) tree: Value,
    pub(super) drawn: Drawn,
}

/// The latest frame the window drew.
#[derive(Default)]
pub(super) struct Frames {
    latest: Mutex<Option<Arc<Frame>>>,
    drawn: Condvar,
}

impl Frames {
    /// Keeps what `runtime` has just presented. Runs on the window's thread after each frame
    /// it draws.
    pub(super) fn record(&self, runtime: &rune::Runtime<App>) {
        let tree = runtime.accessibility_json();
        let drawn = stage::drawn(&runtime.elements(), &runtime.state);
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        let serial = latest.as_ref().map_or(1, |frame| frame.serial + 1);
        *latest = Some(Arc::new(Frame {
            serial,
            tree,
            drawn,
        }));
        self.drawn.notify_all();
    }

    pub(super) fn latest(&self) -> Option<Arc<Frame>> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The serial of the latest frame; 0 before the first.
    pub(super) fn serial(&self) -> u64 {
        self.latest().map_or(0, |frame| frame.serial)
    }

    /// Waits up to `limit` for a frame after `serial`. False when none came.
    pub(super) fn wait_past(&self, serial: u64, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if latest.as_ref().is_some_and(|frame| frame.serial > serial) {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            latest = self
                .drawn
                .wait_timeout(latest, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

#[derive(Clone)]
pub(super) struct Link {
    handle: AppHandle<App>,
    pub(super) frames: Arc<Frames>,
}

impl Link {
    pub(super) fn new(handle: AppHandle<App>, frames: Arc<Frames>) -> Self {
        Self { handle, frames }
    }

    /// Runs `read` on the window's thread and returns its answer.
    pub(super) fn ask<T: Send + 'static>(
        &self,
        read: impl FnOnce(&mut App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.run(read, false)
    }

    /// [`Link::ask`] for a change: the window draws a frame after it.
    pub(super) fn change<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut App) -> T + Send + 'static,
    ) -> Result<T, String> {
        self.run(change, true)
    }

    fn run<T: Send + 'static>(
        &self,
        call: impl FnOnce(&mut App) -> T + Send + 'static,
        changes: bool,
    ) -> Result<T, String> {
        let (answer, answered) = mpsc::sync_channel(1);
        self.handle.update_if(move |app, _| {
            // The session may have stopped waiting; the answer is then nobody's.
            let _ = answer.send(call(app));
            changes
        });
        answered
            .recv_timeout(ANSWER_LIMIT)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => format!(
                    "the window's loop did not answer within {} s",
                    ANSWER_LIMIT.as_secs()
                ),
                mpsc::RecvTimeoutError::Disconnected => "the window's loop has ended".to_owned(),
            })
    }

    /// Ends the window's loop, and so the process.
    pub(super) fn quit(&self) {
        self.handle.quit();
    }
}
