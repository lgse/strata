// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

type RefreshPreference = dyn Fn(&gtk::Widget, &ThemeManager);

struct PreferenceListener {
    active: Cell<bool>,
    anchor: glib::WeakRef<gtk::Widget>,
    refresh: Box<RefreshPreference>,
}

pub(super) struct PreferenceChanges {
    latest: RefCell<Preferences>,
    revision: Cell<u64>,
    notifying: Cell<bool>,
    listeners: Rc<RefCell<Vec<Rc<PreferenceListener>>>>,
}

impl PreferenceChanges {
    pub(super) fn new(preferences: Preferences) -> Self {
        Self {
            latest: RefCell::new(preferences),
            revision: Cell::new(0),
            notifying: Cell::new(false),
            listeners: Rc::default(),
        }
    }

    pub(super) fn record(&self, preferences: &Preferences) -> bool {
        if *self.latest.borrow() == *preferences {
            return false;
        }
        self.latest.replace(preferences.clone());
        self.revision.set(self.revision.get().wrapping_add(1));
        true
    }

    pub(super) fn notify(&self, manager: &ThemeManager) {
        if self.notifying.replace(true) {
            return;
        }
        loop {
            let revision = self.revision.get();
            let listeners = self.listeners.borrow().clone();
            notify_live(
                listeners,
                |listener| listener.active.get() && listener.anchor.upgrade().is_some(),
                |listener| {
                    if listener.active.get()
                        && let Some(anchor) = listener.anchor.upgrade()
                    {
                        (listener.refresh)(&anchor, manager);
                    }
                },
            );
            if self.revision.get() == revision {
                break;
            }
        }
        self.notifying.set(false);
    }
}

impl ThemeManager {
    /// Applies the current value immediately, then only changes to that value.
    /// Capture weak references to owned widgets/state; the anchor owns the binding's lifetime.
    pub(crate) fn bind_preference<T: PartialEq + Clone + 'static>(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        read: impl Fn(&Self) -> T + 'static,
        apply: impl Fn(&gtk::Widget, T) + 'static,
    ) {
        let previous = RefCell::new(None);
        let listener = Rc::new(PreferenceListener {
            active: Cell::new(true),
            anchor: anchor.as_ref().downgrade(),
            refresh: Box::new(move |widget, manager| {
                let value = read(manager);
                if previous.borrow().as_ref() == Some(&value) {
                    return;
                }
                previous.replace(Some(value.clone()));
                apply(widget, value);
            }),
        });
        self.changes.listeners.borrow_mut().push(listener.clone());
        let weak_listeners = Rc::downgrade(&self.changes.listeners);
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
        (listener.refresh)(anchor.as_ref(), self);
    }
}

#[cfg(test)]
mod tests;
