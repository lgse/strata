// SPDX-License-Identifier: MIT

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct State {
    reached: bool,
    released: bool,
    on_ui_thread: Option<bool>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

#[derive(Default)]
pub(crate) struct Parked(Shared);

fn state(shared: &Shared) -> MutexGuard<'_, State> {
    shared.0.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Parked {
    pub(crate) fn racer(&self) -> Box<dyn FnMut() + Send> {
        let shared = self.0.clone();
        Box::new(move || {
            let mut parked = state(&shared);
            parked.reached = true;
            parked.on_ui_thread = Some(crate::util::ui_thread::is_current());
            shared.1.notify_all();
            while !parked.released {
                parked = shared
                    .1
                    .wait(parked)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        })
    }

    pub(crate) fn reached(&self) -> bool {
        state(&self.0).reached
    }

    pub(crate) fn reached_on_ui_thread(&self) -> Option<bool> {
        state(&self.0).on_ui_thread
    }

    pub(crate) fn release(&self) {
        state(&self.0).released = true;
        self.0.1.notify_all();
    }
}

impl Drop for Parked {
    fn drop(&mut self) {
        self.release();
    }
}
