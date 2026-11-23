// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use strata_fs::{adapters::LocalActionRunner, services::JobService};

thread_local! {
    /// The process's service, while a window holds it: the interface thread's.
    static SHARED: RefCell<std::rc::Weak<JobService>> = const { RefCell::new(std::rc::Weak::new()) };
}

/// The process's job service, made with the first window that asks for it.
pub(super) fn join() -> Rc<JobService> {
    SHARED.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.upgrade().unwrap_or_else(|| {
            let fresh = JobService::new(LocalActionRunner::new());
            *slot = Rc::downgrade(&fresh);
            fresh
        })
    })
}
