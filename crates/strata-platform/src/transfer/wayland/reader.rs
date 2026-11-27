// SPDX-License-Identifier: MIT

//! Background reader for a separate Wayland connection (`Connection::connect_to_env`, used when
//! data control cannot share winit's `wl_display`).
//!
//! Winit's shared `wl_display` is driven on the UI thread via `rune::watch_display_queue` and
//! never spawns a reader thread.

use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use wayland_backend::client::WaylandError;
use wayland_client::Connection;

use super::super::pipe;

pub(super) struct Reader {
    stopped: Arc<AtomicBool>,
    stop: Option<OwnedFd>,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    pub(super) fn spawn(
        name: &str,
        conn: Connection,
        mut on_events: impl FnMut() + Send + 'static,
        on_fail: impl FnOnce(String) + Send + 'static,
    ) -> io::Result<Reader> {
        let (stop_r, stop_w) = pipe::pipe()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_stopped = stopped.clone();
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let failed = loop {
                    if thread_stopped.load(Ordering::Acquire) {
                        break None;
                    }
                    let Some(guard) = conn.prepare_read() else {
                        // A dead connection keeps its events queued: libwayland dispatches
                        // nothing once it has an error, and wayland-client's dispatch_pending
                        // drops that error. Without this check the loop would spin forever.
                        if let Some(e) = conn.backend().last_error() {
                            break Some(e.to_string());
                        }
                        on_events();
                        continue;
                    };
                    let revents = match pipe::poll(
                        &[
                            (guard.connection_fd(), libc::POLLIN),
                            (stop_r.as_fd(), libc::POLLIN),
                        ],
                        None,
                    ) {
                        Ok(r) => r,
                        Err(e) => break Some(format!("poll: {e}")),
                    };
                    if revents[1] != 0 || thread_stopped.load(Ordering::Acquire) {
                        // Dropping the guard is cancel_read on an intent we hold.
                        drop(guard);
                        break None;
                    }
                    if revents[0] & libc::POLLIN != 0 {
                        match guard.read_without_dispatch() {
                            Ok(()) => {}
                            Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(e) => break Some(e.to_string()),
                        }
                    } else if revents[0] & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                        drop(guard);
                        break Some("the display hung up".to_owned());
                    }
                };
                if let Some(e) = failed {
                    on_fail(e);
                }
            })?;
        Ok(Reader {
            stopped,
            stop: Some(stop_w),
            thread: Some(thread),
        })
    }

    pub(super) fn stop(mut self) {
        self.stopped.store(true, Ordering::Release);
        // Closing the write end makes the read end readable (POLLHUP), which ends the poll.
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
