// SPDX-License-Identifier: MIT

//! `BrowserEvent`s on their way to the interface.
//!
//! A browser observer runs inside GLib's dispatch. The host is then in the middle of `poll` or
//! between two frames, and Rune holds the application state either way. So an observer only
//! queues, and the interface applies the queue in order.
//!
//! What an event changed is read by each part's [`Listener`] while the browser still holds
//! exactly that: by the time the queue is applied, later events may have moved its entries
//! on. So a listener copies what its part needs, and queues an update that carries the copy.
//!
//! The callback of any other service runs inside GLib's dispatch too, and queues through a
//! [`Later`]. Both hosts apply the queue after every dispatch, so it is the one way in that
//! wakes the window: a queue of a part's own, read in its `poll`, waits for a frame.
//!
//! Each tab of the window has a browser and a state for each part of its own, so what is
//! queued says whose it is ([`Queued::tab`]): a tab's browser and the services its parts asked
//! queue for that tab, and the window's own services for the window. The shell applies what
//! is the window's and the shown tab's, and holds the rest for its tab until it is shown.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
};

use rune::Tx;
use strata_fs::app::{Browser, BrowserEvent};

/// A change to the application state, waiting for `poll`. True when the view has to run.
pub(crate) type Update<S> = Box<dyn FnOnce(&mut S, &mut Tx) -> bool>;

/// What reads one part's share of an event out of the browser, and queues what it asks.
pub(crate) type Listener<S> = Box<dyn Fn(&BrowserEvent, &Browser, &mut Heard<'_, S>)>;

/// A tab of the window, as the queue names it: the window's own updates name none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TabKey(pub(crate) u64);

/// What an update was queued by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// A browser's event, with the rows of a listing it copied out of the browser.
    Event { rows: u64 },
    /// A service other than the browser, through a [`Later`].
    Service,
}

/// One queued update, with the tab it is for and what queued it.
pub(crate) struct Queued<S> {
    pub(crate) tab: Option<TabKey>,
    pub(crate) source: Source,
    pub(crate) update: Update<S>,
}

/// What the listeners queued for one event.
struct Batch<S> {
    updates: Vec<Update<S>>,
    copied: u64,
}

/// Where a listener puts what an event asks of the interface.
pub(crate) struct Heard<'a, S>(&'a mut Batch<S>);

impl<S> Heard<'_, S> {
    /// Queues `update` for the next `poll`, after everything queued before it.
    pub(crate) fn then(&mut self, update: impl FnOnce(&mut S, &mut Tx) -> bool + 'static) {
        self.0.updates.push(Box::new(update));
    }

    /// Counts `rows` of a listing copied out of the browser, for the trace line.
    pub(crate) fn copied(&mut self, rows: u64) {
        self.0.copied += rows;
    }
}

struct Queue<S> {
    updates: VecDeque<Queued<S>>,
    /// How many updates have been queued for each tab, ever.
    queued: HashMap<TabKey, u64>,
    heard: u64,
    copied: u64,
}

pub(crate) struct Inbox<S>(Rc<RefCell<Queue<S>>>);

impl<S> Clone for Inbox<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Where a part queues an update from the callback of a service other than `Browser`: a
/// volume monitor, a thumbnailer, a session store. Call it on the interface thread, from
/// what GLib dispatches there. The update runs in the next `poll`, in its turn among what
/// the browser's events queued, once its tab is the one shown.
pub(crate) struct Later<S> {
    inbox: Inbox<S>,
    tab: Option<TabKey>,
}

impl<S> Clone for Later<S> {
    fn clone(&self) -> Self {
        Self {
            inbox: self.inbox.clone(),
            tab: self.tab,
        }
    }
}

/// A [`Later`] that does not keep its window's queue: what a process-wide service holds for
/// each window, so that a closed window's queue goes with it.
pub(crate) struct WeakLater<S> {
    inbox: Weak<RefCell<Queue<S>>>,
    tab: Option<TabKey>,
}

impl<S> WeakLater<S> {
    /// The way in again, while the window's queue is there.
    pub(crate) fn upgrade(&self) -> Option<Later<S>> {
        self.inbox.upgrade().map(|inbox| Later {
            inbox: Inbox(inbox),
            tab: self.tab,
        })
    }

    pub(crate) fn is_live(&self) -> bool {
        self.inbox.strong_count() > 0
    }
}

impl<S: 'static> Later<S> {
    /// A way in that does not keep the queue.
    pub(crate) fn downgrade(&self) -> WeakLater<S> {
        WeakLater {
            inbox: Rc::downgrade(&self.inbox.0),
            tab: self.tab,
        }
    }

    /// Queues `update`, after everything queued before it. It returns true when the view has
    /// to run.
    pub(crate) fn then(&self, update: impl FnOnce(&mut S, &mut Tx) -> bool + 'static) {
        let mut queue = self.inbox.0.borrow_mut();
        if let Some(tab) = self.tab {
            *queue.queued.entry(tab).or_default() += 1;
        }
        queue.updates.push_back(Queued {
            tab: self.tab,
            source: Source::Service,
            update: Box::new(update),
        });
    }
}

impl<S: 'static> Inbox<S> {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(Queue {
            updates: VecDeque::new(),
            queued: HashMap::new(),
            heard: 0,
            copied: 0,
        })))
    }

    /// The way into this queue for what is not a browser event, for the window.
    pub(crate) fn later(&self) -> Later<S> {
        self.later_for(None)
    }

    /// The way into this queue for what is not a browser event, for `tab`, or for the window.
    pub(crate) fn later_for(&self, tab: Option<TabKey>) -> Later<S> {
        Later {
            inbox: self.clone(),
            tab,
        }
    }

    /// Lets `listeners` fill the inbox from every event of `browser`, each in turn.
    #[cfg(test)]
    pub(crate) fn observe(&self, browser: &Rc<Browser>, listeners: Vec<Listener<S>>) {
        self.observe_for(browser, listeners, None);
    }

    /// Lets `listeners` fill the inbox for `tab` from every event of `browser`, each in turn.
    pub(crate) fn observe_for(
        &self,
        browser: &Rc<Browser>,
        listeners: Vec<Listener<S>>,
        tab: Option<TabKey>,
    ) {
        let queue = self.clone();
        let observed = Rc::downgrade(browser);
        browser.observe(move |event| {
            if let Some(browser) = observed.upgrade() {
                queue.hear(event, &browser, &listeners, tab);
            }
        });
    }

    /// Lets each of `listeners` read `event`, in order, and queues what they ask in that
    /// order.
    fn hear(
        &self,
        event: &BrowserEvent,
        browser: &Browser,
        listeners: &[Listener<S>],
        tab: Option<TabKey>,
    ) {
        let mut batch = Batch {
            updates: Vec::new(),
            copied: 0,
        };
        for listener in listeners {
            listener(event, browser, &mut Heard(&mut batch));
        }
        let mut queue = self.0.borrow_mut();
        queue.heard += 1;
        queue.copied += batch.copied;
        if let Some(tab) = tab {
            *queue.queued.entry(tab).or_default() += batch.updates.len() as u64;
        }
        // The rows are the event's: its first update carries them for the tab's count.
        let mut rows = batch.copied;
        queue
            .updates
            .extend(batch.updates.into_iter().map(|update| Queued {
                tab,
                source: Source::Event {
                    rows: std::mem::take(&mut rows),
                },
                update,
            }));
    }

    pub(crate) fn pop(&self) -> Option<Queued<S>> {
        self.0.borrow_mut().updates.pop_front()
    }

    /// How many updates have been queued for `tab`, and how many of them still wait here.
    pub(crate) fn queued_for(&self, tab: TabKey) -> (u64, u64) {
        let queue = self.0.borrow();
        let waiting = queue
            .updates
            .iter()
            .filter(|queued| queued.tab == Some(tab));
        let queued = queue.queued.get(&tab).copied().unwrap_or(0);
        (queued, waiting.count() as u64)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.borrow().updates.is_empty()
    }

    /// How many events the browser has sent, shown or not.
    pub(crate) fn heard(&self) -> u64 {
        self.0.borrow().heard
    }

    /// How many rows of listings those events had copied out of the browser.
    pub(crate) fn copied(&self) -> u64 {
        self.0.borrow().copied
    }
}
