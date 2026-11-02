// SPDX-License-Identifier: MIT

//! Widget bindings that follow values the theme manager derives, each refreshed
//! only when its value changes and dropped with its anchor widget.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::{super::preferences::notify_live, ThemeManager};

type ThemeRefreshPreference = dyn Fn(&gtk::Widget, &ThemeManager);

struct ThemePreferenceListener {
    active: Cell<bool>,
    anchor: glib::WeakRef<gtk::Widget>,
    refresh: Box<ThemeRefreshPreference>,
}

#[derive(Default)]
pub(super) struct ThemeListeners {
    notifying: Cell<bool>,
    listeners: Rc<RefCell<Vec<Rc<ThemePreferenceListener>>>>,
}

/// A refresh that applies `read`'s value only when it differs from the last one applied.
pub(super) fn on_change<T: PartialEq + Clone + 'static>(
    read: impl Fn(&ThemeManager) -> T + 'static,
    apply: impl Fn(&gtk::Widget, T) + 'static,
) -> Box<ThemeRefreshPreference> {
    let previous = RefCell::new(None);
    Box::new(move |widget, manager| {
        let value = read(manager);
        if previous.borrow().as_ref() == Some(&value) {
            return;
        }
        previous.replace(Some(value.clone()));
        apply(widget, value);
    })
}

impl ThemeListeners {
    pub(super) fn bind(
        &self,
        manager: &ThemeManager,
        anchor: &impl IsA<gtk::Widget>,
        refresh: Box<ThemeRefreshPreference>,
    ) {
        let listener = Rc::new(ThemePreferenceListener {
            active: Cell::new(true),
            anchor: anchor.as_ref().downgrade(),
            refresh,
        });
        self.listeners.borrow_mut().push(listener.clone());
        let weak_listeners = Rc::downgrade(&self.listeners);
        let weak_listener = Rc::downgrade(&listener);
        anchor.connect_destroy(move |_| {
            if let Some(listener) = weak_listener.upgrade() {
                listener.active.set(false);
            }
            if let Some(listeners) = weak_listeners.upgrade() {
                listeners.borrow_mut().retain(|candidate| {
                    !std::rc::Weak::ptr_eq(&Rc::downgrade(candidate), &weak_listener)
                });
            }
        });
        (listener.refresh)(anchor.as_ref(), manager);
    }

    pub(super) fn notify(&self, manager: &ThemeManager) {
        if self.notifying.replace(true) {
            return;
        }
        notify_live(
            &self.listeners,
            |listener| listener.active.get() && listener.anchor.upgrade().is_some(),
            |listener| {
                if listener.active.get()
                    && let Some(anchor) = listener.anchor.upgrade()
                {
                    (listener.refresh)(&anchor, manager);
                }
            },
        );
        self.notifying.set(false);
    }
}
