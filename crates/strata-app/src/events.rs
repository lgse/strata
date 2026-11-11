// SPDX-License-Identifier: MIT

//! `BrowserEvent`s on their way to the interface.
//!
//! A browser observer runs inside GLib's dispatch. The host is then in the middle of `poll` or
//! between two frames, and Rune holds the application state either way. So an observer only
//! queues, and `poll` applies the queue at the start of the next frame.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use strata_fs::app::{Browser, BrowserEvent};

/// What a `BrowserEvent` asks of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// The columns or the active one changed, so the window may show another location.
    Path,
}

#[derive(Default)]
struct Queue {
    changes: VecDeque<Change>,
    heard: u64,
}

#[derive(Clone, Default)]
pub(crate) struct Inbox(Rc<RefCell<Queue>>);

impl Inbox {
    pub(crate) fn observing(browser: &Browser) -> Self {
        let inbox = Self::default();
        let queue = inbox.0.clone();
        browser.observe(move |event| {
            let mut queue = queue.borrow_mut();
            queue.heard += 1;
            if let Some(change) = change_for(event)
                && queue.changes.back() != Some(&change)
            {
                queue.changes.push_back(change);
            }
        });
        inbox
    }

    pub(crate) fn pop(&self) -> Option<Change> {
        self.0.borrow_mut().changes.pop_front()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.borrow().changes.is_empty()
    }

    /// How many events the browser has sent, shown or not.
    pub(crate) fn heard(&self) -> u64 {
        self.0.borrow().heard
    }
}

fn change_for(event: &BrowserEvent) -> Option<Change> {
    match event {
        BrowserEvent::Reset
        | BrowserEvent::ColumnsTruncated { .. }
        | BrowserEvent::ColumnAdded { .. }
        | BrowserEvent::ColumnsRelocated { .. }
        | BrowserEvent::FocusChanged { .. }
        | BrowserEvent::SelectionSetChanged { .. } => Some(Change::Path),
        _ => None,
    }
}
