//! Questions from a GLib-side handler, answered on Rune's loop.

use std::future::Future;

use futures_channel::oneshot;
use rune::Waker;

use super::channel::{Emitter, Inbox};

/// The GLib side of [`Glib::requests`](super::Glib::requests). Cloneable, so each signal emission
/// can take its own.
pub struct Asker<Q, A> {
    emitter: Emitter<Request<Q, A>>,
}

/// The app's side of [`Glib::requests`](super::Glib::requests), kept in state and drained in
/// `poll`. Dropping it answers every request still queued, and every later one, with `None`.
pub struct Requests<Q, A> {
    inbox: Inbox<Request<Q, A>>,
}

/// One question from the GLib side. Answer it with [`respond`](Request::respond); dropping it
/// unanswered resolves the asker's [`ask`](Asker::ask) to `None` (for a mount operation: abort).
#[must_use = "dropping a Request answers it with None"]
pub struct Request<Q, A> {
    question: Q,
    reply: oneshot::Sender<A>,
}

impl<Q, A> Asker<Q, A> {
    pub(super) fn new(emitter: Emitter<Request<Q, A>>) -> Self {
        Asker { emitter }
    }

    /// Queues `question` for the app and resolves to its answer, or to `None` when the request
    /// is dropped unanswered or the [`Requests`] is gone. Await it in a future on the GLib thread
    /// (see [`spawn_local`](super::spawn_local)); it never blocks the loop.
    ///
    /// Dropping the future before it resolves withdraws the request: the app is woken, and
    /// [`Request::is_withdrawn`] turns true, so a prompt can close.
    pub fn ask(&self, question: Q) -> impl Future<Output = Option<A>> + use<Q, A> {
        let (reply, answer) = oneshot::channel();
        // Refused by a closed inbox, the request drops here and the answer resolves to None.
        let queued = self.emitter.emit(Request { question, reply });
        // Made out here, not in the async block, so it drops with a future never polled too.
        let withdrawal = Withdrawal {
            waker: queued.then(|| self.emitter.waker()),
        };
        async move {
            let answer = answer.await.ok();
            withdrawal.disarm();
            answer
        }
    }
}

/// Wakes the app when an [`ask`](Asker::ask) future drops before it resolves.
struct Withdrawal {
    waker: Option<Waker>,
}

impl Withdrawal {
    fn disarm(mut self) {
        self.waker = None;
    }
}

impl Drop for Withdrawal {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker();
        }
    }
}

impl<Q, A> Clone for Asker<Q, A> {
    fn clone(&self) -> Self {
        Asker {
            emitter: self.emitter.clone(),
        }
    }
}

impl<Q, A> Requests<Q, A> {
    pub(super) fn new(inbox: Inbox<Request<Q, A>>) -> Self {
        Requests { inbox }
    }

    /// Takes every request queued so far, oldest first.
    pub fn drain(&self) -> std::vec::IntoIter<Request<Q, A>> {
        self.inbox.drain()
    }

    /// Whether no request is waiting.
    pub fn is_empty(&self) -> bool {
        self.inbox.is_empty()
    }
}

impl<Q, A> Request<Q, A> {
    pub fn question(&self) -> &Q {
        &self.question
    }

    /// Answers the asker. Returns false when it stopped waiting (its future was dropped).
    pub fn respond(self, answer: A) -> bool {
        self.reply.send(answer).is_ok()
    }

    /// Whether the asker stopped waiting: its [`ask`](Asker::ask) future was dropped, and an
    /// answer would go nowhere. The app is woken when that happens.
    pub fn is_withdrawn(&self) -> bool {
        self.reply.is_canceled()
    }
}
