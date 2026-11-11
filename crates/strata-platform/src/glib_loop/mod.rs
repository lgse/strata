// SPDX-License-Identifier: MIT

//! A GLib main context dispatched on Rune's own thread, without polling.
//!
//! Rune lacks: a GLib main loop. Rune runs winit's event loop and never iterates a
//! `GMainContext`, so sources attached to one never dispatch in a Rune app. The
//! [`glib_bridge`](crate::glib_bridge) answers that with a context on a thread of its own,
//! which suits `Send` work. This module is for code that must stay on the interface thread:
//! state behind `Rc` that schedules on its thread's context and is read from handlers.
//!
//! Rune issue: [#139](https://github.com/anomalyco/rune/issues/139) decides how the app is
//! told. While a Wayland window is covered, a `Waker` call never runs `poll`, and a
//! `Tx::wake_after` timer runs every 250 ms whatever it asked for; `AppHandle::update_if`
//! runs on time, covered or not. `tests/session/wake.rs` measures all three.
//!
//! Work package: V5, `docs/rune/slice/V5-shell-and-loop.md`.
//!
//! # The shape
//!
//! This is GLib's own recipe for a foreign loop, with the wait moved off the thread. The
//! interface thread asks the context what it waits for (`g_main_context_prepare` and
//! `g_main_context_query`: file descriptors and a timeout). A helper thread named
//! [`THREAD_NAME`] waits for exactly that and calls `notify`. The app then calls
//! [`GlibLoop::pump`] on the interface thread, which dispatches what is ready and asks again.
//!
//! ```ignore
//! // Before `App::run`, on the thread that owns the context:
//! let handle = app.handle();
//! let glib = GlibLoop::start(glib::MainContext::default(), move || {
//!     handle.update_if(|state, tx| pump(state, tx))
//! })?;
//!
//! // `pump` is also the first thing `AppSpec::poll` does.
//! fn pump(state: &mut State, tx: &mut rune::Tx) -> bool {
//!     state.glib.pump(Duration::from_millis(8));
//!     // … apply what the dispatched callbacks queued; true when the view must run
//! }
//! ```
//!
//! # What the app owes it
//!
//! - **The calling thread owns the context** for as long as the loop lives, and every `pump`
//!   runs there. [`GlibLoop`] is neither `Send` nor `Sync`.
//! - **`notify` only hands over.** It runs on the helper thread. Send the pump to the
//!   interface thread with `AppHandle::update_if`, never with the `Waker` alone.
//! - **Pump from `poll` too.** A source attached from another thread wakes the helper, because
//!   GLib signals a context owned elsewhere. A source the interface thread attaches itself,
//!   in a handler, does not: the helper still waits for what the last `pump` asked. Rune runs
//!   `poll` after handlers, and the pump there asks again.
//! - **Callbacks don't touch the app state.** `pump` dispatches while Rune holds the state, in
//!   `poll` or an update. A callback queues what it has; the code after `pump` applies it.

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
    /// Starts the helper thread for `context`, which the calling thread must own. Nothing is
    /// waited for until the first [`pump`](Self::pump).
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

    /// Dispatches the sources that are ready, for about `budget` at most, then tells the
    /// helper what the context waits for now. Returns whether anything was dispatched.
    ///
    /// A pass that runs out of budget leaves sources ready; the helper then notifies again at
    /// once, so the caller's loop runs between two passes.
    pub fn pump(&self, budget: Duration) -> bool {
        let started = Instant::now();
        let mut dispatched = false;
        while self.context.iteration(false) {
            dispatched = true;
            if started.elapsed() >= budget {
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
