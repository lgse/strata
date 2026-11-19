// SPDX-License-Identifier: MIT

//! The process's one [`JobService`], as GTK's `SHARED_JOBS` in `src/ui/jobs.rs` is: "Jobs must
//! survive navigation and individual window closure". Every window's `CustomActions` joins it
//! while one is open (a thread-local `Weak`, as `crate::folder_views` shares its store), keeps
//! its own observer of it for its Jobs dashboard, and the last window's close guard asks it
//! (`crate::shell::close_guard`).
//!
//! GTK pumps the service from one process-wide GLib timer (`install_pump`). Here every window
//! pumps it from its poll while jobs are active: a job's change reaches every window through its
//! observer, which wakes the window, whose poll then keeps waking itself and its timer until no
//! job is left (`CustomActions::poll`). So a window that closes leaves the others pumping.

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
