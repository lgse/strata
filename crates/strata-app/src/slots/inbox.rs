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

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use rune::Tx;
use strata_fs::app::{Browser, BrowserEvent};

/// A change to the application state, waiting for `poll`. True when the view has to run.
pub(crate) type Update<S> = Box<dyn FnOnce(&mut S, &mut Tx) -> bool>;

/// What reads one part's share of an event out of the browser, and queues what it asks.
pub(crate) type Listener<S> = Box<dyn Fn(&BrowserEvent, &Browser, &mut Heard<'_, S>)>;

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
    updates: VecDeque<Update<S>>,
    heard: u64,
    copied: u64,
}

pub(crate) struct Inbox<S>(Rc<RefCell<Queue<S>>>);

impl<S> Clone for Inbox<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S: 'static> Inbox<S> {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Queue {
            updates: VecDeque::new(),
            heard: 0,
            copied: 0,
        })))
    }

    /// An inbox that `listeners` fill from every event of `browser`, each in turn.
    pub(crate) fn observing(browser: &Rc<Browser>, listeners: Vec<Listener<S>>) -> Self {
        let inbox = Self::new();
        let queue = inbox.clone();
        let observed = Rc::downgrade(browser);
        browser.observe(move |event| {
            if let Some(browser) = observed.upgrade() {
                queue.hear(event, &browser, &listeners);
            }
        });
        inbox
    }

    /// Lets each of `listeners` read `event`, in order, and queues what they ask in that
    /// order. The queue is not held while they read: a listener may call the browser.
    pub(crate) fn hear(&self, event: &BrowserEvent, browser: &Browser, listeners: &[Listener<S>]) {
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
        queue.updates.extend(batch.updates);
    }

    /// Queues `update` from outside an event.
    pub(crate) fn push(&self, update: impl FnOnce(&mut S, &mut Tx) -> bool + 'static) {
        self.0.borrow_mut().updates.push_back(Box::new(update));
    }

    pub(crate) fn pop(&self) -> Option<Update<S>> {
        self.0.borrow_mut().updates.pop_front()
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
