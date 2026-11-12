// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{ListenerGuard, Listeners};

type Callback = Rc<dyn Fn()>;

#[test]
fn notifies_every_active_listener() {
    let listeners: Rc<Listeners<Callback>> = Rc::new(Listeners::new());
    let calls = Rc::new(Cell::new(0));
    let counter = |calls: &Rc<Cell<u32>>| -> Callback {
        let calls = calls.clone();
        Rc::new(move || calls.set(calls.get() + 1))
    };
    let first = listeners.add(counter(&calls));
    let second = listeners.add(counter(&calls));
    listeners.notify(|callback| callback());
    assert_eq!(calls.get(), 2);

    drop(first);
    listeners.notify(|callback| callback());
    assert_eq!(calls.get(), 3, "a dropped guard must unsubscribe");
    drop(second);
    assert!(listeners.is_empty());
}

#[test]
fn a_listener_can_unsubscribe_while_being_notified() {
    let listeners: Rc<Listeners<Callback>> = Rc::new(Listeners::new());
    let guard: Rc<RefCell<Option<ListenerGuard<Callback>>>> = Rc::new(RefCell::new(None));
    let removal = guard.clone();
    let registered = listeners.add(Rc::new(move || {
        removal.borrow_mut().take();
    }));
    guard.borrow_mut().replace(registered);
    listeners.notify(|callback| callback());
    assert!(
        listeners.is_empty(),
        "notifying must not hold the entries borrow across callbacks"
    );
}

#[test]
fn a_guard_does_not_keep_the_registry_alive() {
    let listeners: Rc<Listeners<Callback>> = Rc::new(Listeners::new());
    let guard = listeners.add(Rc::new(|| {}));
    let weak = Rc::downgrade(&listeners);
    assert_eq!(Rc::strong_count(&listeners), 1);
    drop(listeners);
    assert!(
        weak.upgrade().is_none(),
        "a guard must not keep the registry alive"
    );
    drop(guard);
}
