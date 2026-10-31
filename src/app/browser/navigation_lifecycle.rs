// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::Browser;

type Observer = Rc<dyn Fn()>;

#[derive(Default)]
pub(super) struct NavigationLifecycle {
    pub(super) pending: Cell<Option<u64>>,
    depth: Cell<usize>,
    observers: RefCell<Vec<Observer>>,
}

pub(super) struct NavigationUpdate<'a>(&'a Browser);

impl Drop for NavigationUpdate<'_> {
    fn drop(&mut self) {
        let lifecycle = &self.0.navigation_lifecycle;
        lifecycle.depth.set(lifecycle.depth.get() - 1);
        if lifecycle.depth.get() == 0 {
            let observers = lifecycle.observers.borrow().clone();
            for observer in observers {
                observer();
            }
        }
    }
}

impl Browser {
    pub(crate) fn observe_navigation(&self, observer: impl Fn() + 'static) {
        self.navigation_lifecycle
            .observers
            .borrow_mut()
            .push(Rc::new(observer));
    }

    pub(crate) fn pending_navigation_generation(&self) -> Option<u64> {
        self.navigation_lifecycle.pending.get()
    }

    pub(crate) fn navigation_update_in_progress(&self) -> bool {
        self.navigation_lifecycle.depth.get() != 0
    }

    pub(super) fn navigation_update(&self) -> NavigationUpdate<'_> {
        let lifecycle = &self.navigation_lifecycle;
        lifecycle.depth.set(lifecycle.depth.get() + 1);
        NavigationUpdate(self)
    }

    pub(super) fn finish_navigation_validation(&self, generation: u64) -> bool {
        if self.navigation_generation() != generation
            || self.pending_navigation_generation() != Some(generation)
        {
            return false;
        }
        self.navigation_lifecycle.pending.set(None);
        true
    }

    pub(super) fn clear_navigation_observers(&self) {
        self.navigation_lifecycle.observers.borrow_mut().clear();
    }
}
