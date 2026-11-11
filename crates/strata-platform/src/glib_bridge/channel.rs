//! A queue from the GLib thread to Rune's loop that wakes the app on every message.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rune::Waker;

/// The sending half of [`Glib::channel`](super::Glib::channel): queues a message for the app's
/// `poll` and calls the waker. Cloneable and `Send`, so a signal handler on the GLib thread can
/// own one.
pub struct Emitter<M> {
    queue: Arc<Queue<M>>,
    waker: Waker,
}

/// The receiving half of [`Glib::channel`](super::Glib::channel), kept in app state and drained in
/// `poll`. Dropping it closes the channel: later messages are dropped as they are sent.
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
    /// Queues `message`, and wakes the app if the queue was empty: a queue with messages in it
    /// already woke the app, and its next drain takes this one too. Returns false, dropping
    /// `message`, once the [`Inbox`] is gone.
    pub fn emit(&self, message: M) -> bool {
        let was_empty = {
            // Checked under the lock the inbox closes with, so nothing is queued after it closes
            // and lingers there (a queued `Request` would keep its asker waiting).
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
        // Dropped outside the lock: a message's own drop (a `Request` answering its asker) may
        // run code that emits again.
        drop(items);
    }
}
