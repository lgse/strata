// SPDX-License-Identifier: MIT

use std::{cell::Cell, marker::PhantomData};

use gtk::glib;

pub(crate) mod stalls;

thread_local! {
    static MARKED: Cell<bool> = const { Cell::new(false) };
    static ALLOWED: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Exception {
    Startup,
    ExitFlush,
    #[cfg(test)]
    TestSetup,
}

pub(crate) fn mark() {
    MARKED.set(true);
}

pub(crate) fn is_current() -> bool {
    MARKED.get() || glib::MainContext::default().is_owner()
}

pub(crate) fn assert_off() {
    debug_assert!(
        ALLOWED.get() > 0 || !is_current(),
        "file calls must not run on the UI thread"
    );
}

#[must_use]
pub(crate) struct Allowed {
    _exception: Exception,
    _thread: PhantomData<*const ()>,
}

pub(crate) fn allow(exception: Exception) -> Allowed {
    ALLOWED.set(ALLOWED.get() + 1);
    Allowed {
        _exception: exception,
        _thread: PhantomData,
    }
}

impl Drop for Allowed {
    fn drop(&mut self) {
        ALLOWED.set(ALLOWED.get() - 1);
    }
}

#[cfg(test)]
mod tests;
