// SPDX-License-Identifier: MIT

//! The offscreen host as the QA mode drives it: every turn lands its motions, and the tree
//! is read once for as long as nothing ran.

use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use super::tree::Snapshot;
use crate::{host::offscreen::Offscreen, shell::App};

/// How long one settle keeps turning a host that never comes to rest. The GTK harness waits
/// this long for a frame.
const SETTLE_LIMIT: Duration = Duration::from_millis(250);

pub(super) struct Stage {
    host: Offscreen,
    /// The tree as of the last turn, read when first asked for.
    seen: Option<Rc<Snapshot>>,
}

impl Stage {
    pub(super) fn new(host: Offscreen) -> Self {
        Self { host, seen: None }
    }

    pub(super) fn app(&self) -> &App {
        self.host.app()
    }

    /// The host, for anything that can change what it shows: the tree is read again after.
    pub(super) fn host(&mut self) -> &mut Offscreen {
        self.seen = None;
        &mut self.host
    }

    pub(super) fn snapshot(&mut self) -> Rc<Snapshot> {
        let host = &self.host;
        self.seen
            .get_or_insert_with(|| {
                Rc::new(Snapshot::read(
                    &host.tree(),
                    &host.app().columns.list_keys(),
                ))
            })
            .clone()
    }

    /// One turn of the host. A turn that did something lands every motion at its end
    /// (`Runtime::settle_all`): `Runtime::frame` reports work for some hundred milliseconds
    /// after a key otherwise, with the look's animations off too.
    pub(super) fn turn(&mut self) -> bool {
        let host = self.host();
        let worked = host.turn();
        if worked {
            host.runtime().settle_all();
        }
        worked
    }

    /// Turns until neither GLib nor Rune has anything ready. A timer still running, such as
    /// the wait before a child column opens, does not hold it up: a scenario waits for that.
    pub(super) fn settle(&mut self) {
        let deadline = Instant::now() + SETTLE_LIMIT;
        while self.turn() && Instant::now() < deadline {}
    }

    /// Blocks until GLib has work, a Rune timer is due, or `limit` has passed.
    pub(super) fn wait(&mut self, limit: Duration) {
        self.host().wait(limit);
    }

    /// Lets the application run for `time`.
    pub(super) fn sleep(&mut self, time: Duration) {
        let deadline = Instant::now() + time;
        loop {
            let worked = self.turn();
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            if left.is_zero() {
                return;
            }
            if !worked {
                self.wait(left);
            }
        }
    }
}
