//! A future's output, landed for `poll`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rune::Waker;

use super::registry::{self, Key};
use super::thread::Glib;

/// Where a [`Task`] stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// Queued or running on the GLib thread.
    Pending,
    /// The output landed (and may have been taken).
    Done,
    /// It will never land: the future panicked, or the bridge stopped before it finished.
    Stopped,
}

/// The output of a future run by [`Glib::spawn`], kept in app state and read in `poll`. The
/// waker fires once when it lands or stops.
///
/// Dropping a pending task cancels it: its future is dropped on the GLib thread, which cancels
/// the GIO call it awaits.
#[must_use = "dropping a Task cancels it"]
pub struct Task<T> {
    slot: Arc<Slot<T>>,
    key: Key,
    glib: Glib,
}

struct Slot<T> {
    state: Mutex<SlotState<T>>,
    cancelled: AtomicBool,
}

enum SlotState<T> {
    Pending,
    Ready(T),
    Taken,
    Stopped,
}

impl<T> Slot<T> {
    fn lock(&self) -> MutexGuard<'_, SlotState<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Settles a pending slot, and says whether it did.
    fn settle(&self, to: SlotState<T>) -> bool {
        let mut state = self.lock();
        if matches!(*state, SlotState::Pending) {
            *state = to;
            true
        } else {
            false
        }
    }
}

/// Carries a task's output to its [`Task`]. Dropped without delivering (the future panicked or
/// was dropped at shutdown), it settles the task as stopped.
pub(super) struct Delivery<T> {
    slot: Arc<Slot<T>>,
    waker: Waker,
    delivered: bool,
}

impl<T> Delivery<T> {
    /// Whether the [`Task`] was dropped: the work need not start.
    pub(super) fn is_cancelled(&self) -> bool {
        self.slot.cancelled.load(Ordering::Acquire)
    }

    /// Settles the task as stopped without waking: for work refused before it was queued.
    pub(super) fn refuse(mut self) {
        self.delivered = true;
        self.slot.settle(SlotState::Stopped);
    }

    pub(super) fn deliver(mut self, output: T) {
        self.delivered = true;
        self.finish(SlotState::Ready(output));
    }

    fn finish(&self, to: SlotState<T>) {
        // A cancelled task's owner is gone: nobody to wake.
        if self.slot.settle(to) && !self.slot.cancelled.load(Ordering::Acquire) {
            (self.waker)();
        }
    }
}

impl<T> Drop for Delivery<T> {
    fn drop(&mut self) {
        if !self.delivered {
            self.finish(SlotState::Stopped);
        }
    }
}

impl<T: Send + 'static> Task<T> {
    /// A pending task under `key`, and the delivery that settles it.
    pub(super) fn new(glib: &Glib, key: Key, waker: Waker) -> (Self, Delivery<T>) {
        let slot = Arc::new(Slot {
            state: Mutex::new(SlotState::Pending),
            cancelled: AtomicBool::new(false),
        });
        let delivery = Delivery {
            slot: slot.clone(),
            waker,
            delivered: false,
        };
        let task = Task {
            slot,
            key,
            glib: glib.clone(),
        };
        (task, delivery)
    }

    /// The output, once, after it lands.
    pub fn take(&self) -> Option<T> {
        let mut state = self.slot.lock();
        match std::mem::replace(&mut *state, SlotState::Taken) {
            SlotState::Ready(output) => Some(output),
            other => {
                *state = other;
                None
            }
        }
    }

    pub fn state(&self) -> TaskState {
        match *self.slot.lock() {
            SlotState::Pending => TaskState::Pending,
            SlotState::Ready(_) | SlotState::Taken => TaskState::Done,
            SlotState::Stopped => TaskState::Stopped,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.state() == TaskState::Pending
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if !matches!(*self.slot.lock(), SlotState::Pending) {
            return;
        }
        self.slot.cancelled.store(true, Ordering::Release);
        let key = self.key;
        self.glib.run(move || registry::abort(key));
    }
}
