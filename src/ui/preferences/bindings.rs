// SPDX-License-Identifier: MIT

use super::*;

type RefreshPreference = dyn Fn(&gtk::Widget, &PreferenceManager);

struct PreferenceListener {
    active: Cell<bool>,
    anchor: glib::WeakRef<gtk::Widget>,
    refresh: Box<RefreshPreference>,
}

/// Keeps the live listener list for one manager's anchored preference bindings.
pub(in crate::ui) struct PreferenceChanges {
    latest: RefCell<Preferences>,
    revision: Cell<u64>,
    notifying: Cell<bool>,
    listeners: Rc<RefCell<Vec<Rc<PreferenceListener>>>>,
    observers: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl PreferenceChanges {
    pub(in crate::ui) fn new(preferences: Preferences) -> Self {
        Self {
            latest: RefCell::new(preferences),
            revision: Cell::new(0),
            notifying: Cell::new(false),
            listeners: Rc::default(),
            observers: RefCell::default(),
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

    /// Rebases the last-published snapshot without publishing a change, so
    /// startup repairs are the baseline rather than the next notification.
    pub(super) fn reset_baseline(&self, preferences: &Preferences) {
        self.latest.replace(preferences.clone());
    }

    #[cfg(test)]
    pub(super) fn listener_count(&self) -> usize {
        self.listeners.borrow().len()
    }

    pub(super) fn observe(&self, observer: Rc<dyn Fn()>) {
        self.observers.borrow_mut().push(observer);
    }

    pub(super) fn bind<T: PartialEq + Clone + 'static>(
        &self,
        manager: &PreferenceManager,
        anchor: &impl IsA<gtk::Widget>,
        read: impl Fn(&PreferenceManager) -> T + 'static,
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
        self.listeners.borrow_mut().push(listener.clone());
        let weak_listeners = Rc::downgrade(&self.listeners);
        let weak_listener = Rc::downgrade(&listener);
        anchor.connect_destroy(move |_| {
            if let Some(listener) = weak_listener.upgrade() {
                listener.active.set(false);
            }
            if let Some(listeners) = weak_listeners.upgrade() {
                retain_live(&listeners, |candidate| candidate.active.get());
            }
        });
        (listener.refresh)(anchor.as_ref(), manager);
    }

    pub(super) fn release_within(&self, root: &gtk::Widget) {
        let released: Vec<_> = {
            let mut listeners = self.listeners.borrow_mut();
            let (released, kept) =
                std::mem::take(&mut *listeners)
                    .into_iter()
                    .partition(|listener| {
                        listener
                            .anchor
                            .upgrade()
                            .is_some_and(|anchor| &anchor == root || anchor.is_ancestor(root))
                    });
            *listeners = kept;
            released
        };
        for listener in &released {
            listener.active.set(false);
        }
    }

    /// Publishes a change kept outside `Preferences`, such as folder settings,
    /// so that a notification already running repeats for it.
    pub(super) fn notify_unrecorded(&self, manager: &PreferenceManager) {
        self.revision.set(self.revision.get().wrapping_add(1));
        self.notify(manager);
    }

    pub(super) fn notify(&self, manager: &PreferenceManager) {
        if self.notifying.replace(true) {
            return;
        }
        loop {
            let revision = self.revision.get();
            let observers = self.observers.borrow().clone();
            for observer in observers {
                observer();
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
            if self.revision.get() == revision {
                break;
            }
        }
        self.notifying.set(false);
    }
}

pub(in crate::ui) fn notify_live<T: Clone>(
    listeners: &RefCell<Vec<T>>,
    is_live: impl Fn(&T) -> bool,
    run: impl Fn(&T),
) {
    retain_live(listeners, &is_live);
    let live = listeners.borrow().clone();
    for entry in &live {
        if is_live(entry) {
            run(entry);
        }
    }
}

fn retain_live<T>(listeners: &RefCell<Vec<T>>, is_live: impl Fn(&T) -> bool) {
    // Dropping a listener can destroy another anchor and reenter this registry.
    loop {
        let previous = std::mem::take(&mut *listeners.borrow_mut());
        let count = previous.len();
        let mut kept: Vec<_> = previous
            .into_iter()
            .filter(|entry| is_live(entry))
            .collect();
        let removed = kept.len() != count;
        {
            let mut current = listeners.borrow_mut();
            kept.append(&mut *current);
            *current = kept;
        }
        if !removed {
            break;
        }
    }
}

#[cfg(test)]
mod tests;
