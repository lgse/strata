// SPDX-License-Identifier: MIT

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
