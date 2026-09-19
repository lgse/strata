// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

#[cfg(test)]
mod tests;

struct ListenerEntry<T> {
    id: u64,
    callback: T,
}

pub(crate) struct Listeners<T> {
    entries: RefCell<Vec<ListenerEntry<T>>>,
    next_id: Cell<u64>,
}

impl<T> Default for Listeners<T> {
    fn default() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
            next_id: Cell::new(0),
        }
    }
}

impl<T> Listeners<T> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add(self: &Rc<Self>, callback: T) -> ListenerGuard<T> {
        let id = self.next_id.get().wrapping_add(1);
        self.next_id.set(id);
        self.entries
            .borrow_mut()
            .push(ListenerEntry { id, callback });
        ListenerGuard {
            id,
            listeners: Rc::downgrade(self),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }

    // Callbacks may unsubscribe themselves; release the borrow before dispatch.
    pub(crate) fn notify(&self, run: impl Fn(&T))
    where
        T: Clone,
    {
        let callbacks: Vec<T> = self
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.callback.clone())
            .collect();
        for callback in callbacks {
            run(&callback);
        }
    }
}

pub(crate) struct ListenerGuard<T> {
    id: u64,
    listeners: Weak<Listeners<T>>,
}

impl<T> std::fmt::Debug for ListenerGuard<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ListenerGuard")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<T> Drop for ListenerGuard<T> {
    fn drop(&mut self) {
        if let Some(listeners) = self.listeners.upgrade() {
            listeners
                .entries
                .borrow_mut()
                .retain(|entry| entry.id != self.id);
        }
    }
}
