// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::{
    cell::Cell,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

struct Lookup {
    index: usize,
    ready: Rc<Vec<Cell<bool>>>,
    active: Rc<Cell<usize>>,
}

impl Future for Lookup {
    type Output = usize;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<usize> {
        if self.ready[self.index].get() {
            Poll::Ready(self.index)
        } else {
            Poll::Pending
        }
    }
}

impl Drop for Lookup {
    fn drop(&mut self) {
        self.active.set(self.active.get() - 1);
    }
}

#[test]
fn batch_bounds_concurrency_and_preserves_order() {
    let count = MAX_RESTORE_LOOKUPS * 2 + 3;
    let ready = Rc::new((0..count).map(|_| Cell::new(false)).collect::<Vec<_>>());
    let active = Rc::new(Cell::new(0));
    let started = Cell::new(0);
    let mut batch = Box::pin(resolve_batch((0..count).collect(), |index| {
        active.set(active.get() + 1);
        assert!(active.get() <= MAX_RESTORE_LOOKUPS);
        started.set(started.get() + 1);
        Lookup {
            index,
            ready: ready.clone(),
            active: active.clone(),
        }
    }));
    let mut context = Context::from_waker(Waker::noop());
    assert!(batch.as_mut().poll(&mut context).is_pending());
    assert_eq!(started.get(), MAX_RESTORE_LOOKUPS);
    ready[MAX_RESTORE_LOOKUPS - 1].set(true);
    assert!(batch.as_mut().poll(&mut context).is_pending());
    assert_eq!(started.get(), MAX_RESTORE_LOOKUPS + 1);
    for done in ready.iter() {
        done.set(true);
    }
    assert_eq!(
        batch.as_mut().poll(&mut context),
        Poll::Ready((0..count).collect())
    );
    assert_eq!(active.get(), 0);
}

#[test]
fn cancelling_batch_drops_active_lookups_without_starting_queued_items() {
    let count = MAX_RESTORE_LOOKUPS * 3;
    let active = Rc::new(Cell::new(0));
    let ready = Rc::new((0..count).map(|_| Cell::new(false)).collect::<Vec<_>>());
    let started = Cell::new(0);
    let mut batch = Box::pin(resolve_batch((0..count).collect(), |index| {
        active.set(active.get() + 1);
        started.set(started.get() + 1);
        Lookup {
            index,
            ready: ready.clone(),
            active: active.clone(),
        }
    }));
    assert!(
        batch
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(started.get(), MAX_RESTORE_LOOKUPS);
    drop(batch);
    assert_eq!(active.get(), 0);
    assert_eq!(started.get(), MAX_RESTORE_LOOKUPS);
}
