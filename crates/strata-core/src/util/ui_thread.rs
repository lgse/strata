// SPDX-License-Identifier: MIT

use std::{cell::Cell, marker::PhantomData};

pub mod stalls;

thread_local! {
    static MARKED: Cell<bool> = const { Cell::new(false) };
    static ALLOWED: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug)]
pub enum Exception {
    Startup,
    ExitFlush,
    #[cfg(any(test, feature = "test-support"))]
    TestSetup,
}

pub fn mark() {
    MARKED.set(true);
}

pub fn is_current() -> bool {
    MARKED.get() || glib::MainContext::default().is_owner()
}

pub fn assert_off() {
    debug_assert!(
        ALLOWED.get() > 0 || !is_current(),
        "file calls must not run on the UI thread"
    );
}

#[must_use]
pub struct Allowed {
    _exception: Exception,
    _thread: PhantomData<*const ()>,
}

pub fn allow(exception: Exception) -> Allowed {
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

// `assert_off` is a `debug_assert!`, so its tests mean nothing in release builds.
#[cfg(all(test, debug_assertions))]
mod tests;
