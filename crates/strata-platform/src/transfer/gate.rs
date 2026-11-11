//! The handshake between a Wayland reader thread and the UI thread (S2's driver C).
//!
//! The reader asks the UI thread for a turn after each read that queued events, then waits
//! here until the UI has dispatched them, so it doesn't spin on a queue that stays non-empty.
//! It holds no read intent while it waits, so winit reads freely. Stopping releases a waiting
//! reader at once: the UI thread joins the reader during teardown, and must never wait on a
//! turn only it could give.

use std::sync::{Condvar, Mutex, PoisonError};

#[derive(Default)]
pub(crate) struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    turns: u64,
    stopped: bool,
}

impl Gate {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The turns taken so far: read before asking for one, then pass to [`Gate::wait`].
    pub(crate) fn turns(&self) -> u64 {
        self.lock().turns
    }

    /// The UI thread dispatched.
    pub(crate) fn turned(&self) {
        self.lock().turns += 1;
        self.changed.notify_all();
    }

    /// Waits until a turn after `before`, or until stopped. No timeout: a reader parked here
    /// after the event loop ended stays parked, and never touches libwayland again.
    pub(crate) fn wait(&self, before: u64) {
        let mut state = self.lock();
        while state.turns == before && !state.stopped {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(crate) fn stop(&self) {
        self.lock().stopped = true;
        self.changed.notify_all();
    }

    pub(crate) fn stopped(&self) -> bool {
        self.lock().stopped
    }
}

#[cfg(test)]
mod tests;
