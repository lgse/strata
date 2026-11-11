//! What the substrate reports, and how subscribers hear it.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::data::ReadLimits;
use super::error::ReadError;
use super::wayland::DragOffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Selection {
    Clipboard,
    /// The primary selection: data control and X11 only.
    Primary,
}

/// The protocol a selection goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// `wl_data_device` on winit's display: heard only with keyboard focus, and a write needs
    /// a recent input serial. Clipboard only.
    DataDevice,
    /// `ext_data_control_v1`, else `zwlr_data_control_v1`: heard and written without focus.
    DataControl,
    /// X11 `CLIPBOARD` and `PRIMARY`, on a connection of our own.
    X11,
}

/// One source this process offered, until it is cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(u64);

impl SourceId {
    pub(crate) fn next() -> SourceId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        SourceId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// The id logged as `n` by [`SourceId::get`]. For strata-platform's own tests, which
    /// name sources across a process boundary.
    #[doc(hidden)]
    pub fn from_raw(n: u64) -> SourceId {
        SourceId(n)
    }

    /// A number for logs.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A drag-and-drop action, as Wayland numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Copy,
    Move,
    Ask,
}

impl Action {
    pub(crate) fn bits(self) -> u32 {
        match self {
            Action::Copy => 1,
            Action::Move => 2,
            Action::Ask => 4,
        }
    }

    pub(crate) fn from_bits(bits: u32) -> Option<Action> {
        match bits {
            1 => Some(Action::Copy),
            2 => Some(Action::Move),
            4 => Some(Action::Ask),
            _ => None,
        }
    }

    /// The actions in a Wayland action mask.
    pub(crate) fn all_in(bits: u32) -> Vec<Action> {
        [Action::Copy, Action::Move, Action::Ask]
            .into_iter()
            .filter(|a| bits & a.bits() != 0)
            .collect()
    }
}

/// How a read's result comes back: on a worker thread, or the X11 thread.
pub type Done = Box<dyn FnOnce(Result<Vec<u8>, ReadError>) + Send>;

pub(crate) trait ReadOffer: Send + Sync {
    fn mimes(&self) -> &[String];
    fn read(&self, mime: &str, limits: ReadLimits, done: Done);
}

/// Another client's selection (or ours, echoed back): its MIME types, and a read of one.
#[derive(Clone)]
pub struct SelectionOffer(pub(crate) Arc<dyn ReadOffer>);

impl SelectionOffer {
    pub fn mimes(&self) -> &[String] {
        self.0.mimes()
    }

    /// Reads one type. `done` runs once: with the data, on a worker (Wayland) or the X11
    /// thread, so hand the bytes to the app with `AppHandle::update`; or on this thread at once
    /// when the read can't start (a type the offer doesn't list, an offer that is gone, a call
    /// off the UI thread on winit's display). It never runs under a lock of the offer's, so it
    /// may call the offer again. On winit's display, call this on the UI thread.
    pub fn read(
        &self,
        mime: &str,
        limits: ReadLimits,
        done: impl FnOnce(Result<Vec<u8>, ReadError>) + Send + 'static,
    ) {
        if !self.mimes().iter().any(|m| m == mime) {
            done(Err(ReadError::NotOffered));
            return;
        }
        self.0.read(mime, limits, Box::new(done));
    }
}

impl fmt::Debug for SelectionOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SelectionOffer")
            .field(&self.mimes())
            .finish()
    }
}

/// What happened. Every subscriber hears every event.
#[derive(Clone, Debug)]
pub enum Event {
    /// A selection changed hands. `None`: nobody holds it now. It may be this process's own
    /// source, echoed back ([`Transfer::owns`](super::Transfer::owns) says).
    SelectionChanged {
        selection: Selection,
        route: Route,
        offer: Option<SelectionOffer>,
    },
    /// A reader asked one of our sources for a type. Its writer has run, or runs now, once.
    SourceAsked {
        source: SourceId,
        mime: String,
    },
    /// Another client took the selection from one of our sources.
    SourceCancelled {
        source: SourceId,
    },
    /// A drag entered one of the app's windows (Wayland), at window points.
    DragEnter {
        window: String,
        offer: DragOffer,
        x: f64,
        y: f64,
    },
    DragMotion {
        x: f64,
        y: f64,
    },
    DragLeave,
    /// The drop: read the offer, then [`DragOffer::finish`] it.
    Drop,
    /// The drag's source actions, or the action the compositor settled on.
    DragActions {
        source: Vec<Action>,
        chosen: Option<Action>,
    },
    /// A connection failed; its routes are gone.
    Failed {
        message: String,
    },
}

type Notify = Arc<dyn Fn() + Send + Sync>;

/// The substrate's side of a subscription: the inbox weakly, so a dropped [`Subscription`]
/// is forgotten at the next event and hears no more. A notify an emit on another thread had
/// already taken when the subscription dropped may still run once.
struct Subscriber {
    inbox: Weak<Mutex<VecDeque<Event>>>,
    notify: Notify,
}

/// One subscriber's queue of events. Dropping it unsubscribes.
pub struct Subscription {
    inbox: Arc<Mutex<VecDeque<Event>>>,
}

impl Subscription {
    /// Everything heard since the last drain, oldest first.
    pub fn drain(&self) -> Vec<Event> {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

/// Every subscriber, shared by the threads that produce events.
#[derive(Clone, Default)]
pub(crate) struct Subscribers(Arc<Mutex<Vec<Subscriber>>>);

impl Subscribers {
    pub(crate) fn add(&self, notify: Notify) -> Subscription {
        let inbox = Arc::new(Mutex::new(VecDeque::new()));
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Subscriber {
                inbox: Arc::downgrade(&inbox),
                notify,
            });
        Subscription { inbox }
    }

    /// Queues `events` for every subscriber, then wakes each one once.
    pub(crate) fn emit(&self, events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        let notifies: Vec<Notify> = {
            let mut subs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            subs.retain(|sub| sub.inbox.strong_count() > 0);
            subs.iter()
                .filter_map(|sub| {
                    let inbox = sub.inbox.upgrade()?;
                    inbox
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend(events.iter().cloned());
                    Some(sub.notify.clone())
                })
                .collect()
        };
        // Outside the lock: a notify may subscribe or emit.
        for notify in notifies {
            notify();
        }
    }
}

#[cfg(test)]
mod tests;
