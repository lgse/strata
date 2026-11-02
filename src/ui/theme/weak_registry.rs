// SPDX-License-Identifier: MIT

//! Weak lists of the buffers and views a theme change restyles in place.

use std::cell::RefCell;

use gtk::glib::{self, prelude::*};

pub(super) type WeakRegistry<T> = RefCell<Vec<glib::WeakRef<T>>>;

/// Adds `object`, first dropping entries whose object is gone.
pub(super) fn register<T: ObjectType>(registry: &WeakRegistry<T>, object: &T) {
    let mut entries = registry.borrow_mut();
    entries.retain(|entry| entry.upgrade().is_some());
    let weak = glib::WeakRef::new();
    weak.set(Some(object));
    entries.push(weak);
}

/// Runs `apply` on every live object, dropping entries whose object is gone.
pub(super) fn for_each_live<T: ObjectType>(registry: &WeakRegistry<T>, apply: impl Fn(&T)) {
    registry.borrow_mut().retain(|entry| {
        let Some(object) = entry.upgrade() else {
            return false;
        };
        apply(&object);
        true
    });
}
