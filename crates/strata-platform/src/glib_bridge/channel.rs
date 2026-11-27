// SPDX-License-Identifier: MIT

//! A queue from the GLib thread to Rune's loop that wakes the app on every message.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rune::Waker;

pub struct Emitter<M> {
    queue: Arc<Queue<M>>,
    waker: Waker,
}

pub struct Inbox<M> {
    queue: Arc<Queue<M>>,
}

struct Queue<M> {
    state: Mutex<QueueState<M>>,
}

struct QueueState<M> {
    items: VecDeque<M>,
    closed: bool,
}

pub(super) fn channel<M>(waker: Waker) -> (Emitter<M>, Inbox<M>) {
    let queue = Arc::new(Queue {
        state: Mutex::new(QueueState {
            items: VecDeque::new(),
            closed: false,
        }),
    });
    (
        Emitter {
            queue: queue.clone(),
            waker,
        },
        Inbox { queue },
    )
}

impl<M> Queue<M> {
    fn lock(&self) -> MutexGuard<'_, QueueState<M>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<M> Emitter<M> {
    pub fn emit(&self, message: M) -> bool {
        let was_empty = {
            let mut state = self.queue.lock();
            if state.closed {
                return false;
            }
            state.items.push_back(message);
            state.items.len() == 1
        };
        if was_empty {
            (self.waker)();
        }
        true
    }

    pub(super) fn waker(&self) -> Waker {
        self.waker.clone()
    }

    /// Whether the [`Inbox`] is gone.
    pub fn is_closed(&self) -> bool {
        self.queue.lock().closed
    }
}

impl<M> Clone for Emitter<M> {
    fn clone(&self) -> Self {
        Emitter {
            queue: self.queue.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<M> Inbox<M> {
    /// Takes every message queued so far, oldest first.
    pub fn drain(&self) -> std::vec::IntoIter<M> {
        let items: Vec<M> = self.queue.lock().items.drain(..).collect();
        items.into_iter()
    }

    /// Whether no message is waiting.
    pub fn is_empty(&self) -> bool {
        self.queue.lock().items.is_empty()
    }
}

impl<M> Drop for Inbox<M> {
    fn drop(&mut self) {
        let items = {
            let mut state = self.queue.lock();
            state.closed = true;
            std::mem::take(&mut state.items)
        };
        drop(items);
    }
}
