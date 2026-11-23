// SPDX-License-Identifier: MIT

use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::thread::JoinHandle;

use wayland_backend::client::WaylandError;
use wayland_client::Connection;

use super::super::gate::Gate;
use super::super::pipe;

pub(super) struct Reader {
    gate: Arc<Gate>,
    stop: Option<OwnedFd>,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    pub(super) fn spawn(
        name: &str,
        conn: Connection,
        gate: Arc<Gate>,
        mut on_events: impl FnMut() + Send + 'static,
        on_fail: impl FnOnce(String) + Send + 'static,
    ) -> io::Result<Reader> {
        let (stop_r, stop_w) = pipe::pipe()?;
        let thread_gate = gate.clone();
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let gate = thread_gate;
                let failed = loop {
                    if gate.stopped() {
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
                    if revents[1] != 0 || gate.stopped() {
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
            gate,
            stop: Some(stop_w),
            thread: Some(thread),
        })
    }

    pub(super) fn stop(mut self) {
        self.gate.stop();
        // Closing the write end makes the read end readable (POLLHUP), which ends the poll.
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
