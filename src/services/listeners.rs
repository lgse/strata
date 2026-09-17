// SPDX-License-Identifier: MIT

//! Small, shared listener registry used by long-lived services.
//!
//! Both the action registry and the job service need "notify these consumers when
//! state changes, and stop when the consumer goes away". Keeping one tested
//! implementation means a window that closes cannot keep a stale callback (or a
//! service) alive through a forgotten unsubscribe.

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

/// Owners of consumer callbacks. Wrap in `Rc` to hand out [`ListenerGuard`]s.
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

    /// Registers `callback`. The subscription ends when the guard drops.
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

    /// Delivers to every current listener, without holding the borrow across
    /// callbacks so a listener may unsubscribe itself.
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

/// Keeps one [`Listeners`] subscription alive until it is dropped.
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
