// SPDX-License-Identifier: MIT

//! GLib main context dispatched on Rune's UI thread without polling.
//!
//! Rune lacks a GLib main loop integration; a helper thread polls the context's fds/timeout and
//! wakes the UI thread via `AppHandle::update_if` rather than `Waker` because under a covered
//! Wayland window `Waker` does not run `poll`
//! ([anomalyco/rune#139](https://github.com/anomalyco/rune/issues/139)).
//!
//! Invariant: the UI thread must own the `MainContext` for the lifetime of [`GlibLoop`], and
//! sources attached on the UI thread itself require `MainContext::wakeup` or a `poll` pump so
//! the waiter re-queries the context.

mod waiter;

use std::io;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use glib::translate::ToGlibPtr as _;

use waiter::{Wait, Waiter};

pub use waiter::THREAD_NAME;

/// A context's wait handed to a helper thread; see the module docs.
pub struct GlibLoop {
    context: glib::MainContext,
    waiter: Waiter,
    _thread: PhantomData<*const ()>,
}

impl GlibLoop {
    pub fn start(
        context: glib::MainContext,
        notify: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        if !context.is_owner() {
            return Err(io::Error::other(
                "the calling thread does not own the GLib context",
            ));
        }
        Ok(Self {
            context,
            waiter: Waiter::start(Box::new(notify))?,
            _thread: PhantomData,
        })
    }

    pub fn pump(&self, budget: Duration) -> bool {
        self.pump_until(budget, || false)
    }

    pub fn pump_until(&self, budget: Duration, mut stop: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        let mut dispatched = false;
        while self.context.iteration(false) {
            dispatched = true;
            if stop() || started.elapsed() >= budget {
                break;
            }
        }
        let (_ready, priority) = self.context.prepare();
        self.waiter.wait_for(query(&self.context, priority));
        dispatched
    }
}

/// What `context` waits for at `priority`, after a `prepare`: `g_main_context_query`.
#[expect(
    unsafe_code,
    reason = "glib 0.22 does not bind g_main_context_query, the only way to learn the file \
              descriptors and timeout a context waits for"
)]
fn query(context: &glib::MainContext, priority: i32) -> Wait {
    let mut fds = vec![
        glib::ffi::GPollFD {
            fd: -1,
            events: 0,
            revents: 0,
        };
        8
    ];
    loop {
        let mut timeout = -1;
        // SAFETY: the context is alive for the call, and its owner is this thread (`start`
        // checked, and `GlibLoop` cannot leave it). `timeout` is a valid out-pointer. `fds`
        // points at `fds.len()` initialized records, the length passed with it, and GLib
        // writes at most that many. Nothing is retained after the call returns.
        let needed = unsafe {
            glib::ffi::g_main_context_query(
                context.to_glib_none().0,
                priority,
                &mut timeout,
                fds.as_mut_ptr(),
                fds.len() as i32,
            )
        };
        let needed = usize::try_from(needed).unwrap_or(0);
        if needed <= fds.len() {
            fds.truncate(needed);
            return Wait { fds, timeout };
        }
        // The array was too small: GLib filled what fit and returned the count it wants.
        fds.resize(
            needed,
            glib::ffi::GPollFD {
                fd: -1,
                events: 0,
                revents: 0,
            },
        );
    }
}

#[cfg(test)]
mod tests;
