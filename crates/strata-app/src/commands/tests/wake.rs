// SPDX-License-Identifier: MIT

//! A command wakes the GLib wait (`commands::wake_glib`), so what it attached to the
//! interface thread's context runs without a frame: on a real window, the windowed host's
//! helper waits on that context (strata-platform's `glib_loop`), and GLib wakes it for no
//! source its owner attaches. Without the wake, Ctrl+Z after a drag-move waited for the next
//! input on the session host (`docs/rune/parity/packages/F5.md`).

use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

use strata_platform::glib_loop::GlibLoop;

use super::{MODULE, On, Outcome, listed, run};
use crate::test_support::hosted;

const BUDGET: Duration = Duration::from_millis(8);
/// How long the helper must stay silent for the context to count as quiet.
const QUIET: Duration = Duration::from_millis(300);
/// How long the wake may take to reach the helper.
const WOKEN: Duration = Duration::from_secs(2);

/// Pumps until the helper has nothing to tell for [`QUIET`].
fn settle(glib: &GlibLoop, notified: &Receiver<()>) {
    glib.pump(BUDGET);
    while notified.recv_timeout(QUIET) != Err(RecvTimeoutError::Timeout) {
        glib.pump(BUDGET);
    }
}

#[test]
fn a_command_wakes_the_glib_wait_without_a_pump() {
    hosted(
        MODULE,
        "wake::a_command_wakes_the_glib_wait_without_a_pump",
        |mut host| {
            let _folder = listed(&mut host);
            let (notify, notified) = mpsc::channel();
            let glib = GlibLoop::start(glib::MainContext::default(), move || {
                let _ = notify.send(());
            })
            .expect("the test's thread owns the default context");
            settle(&glib, &notified);

            let mut ran = None;
            host.runtime()
                .update(|app, tx| ran = Some(run(app, "select-all", On::Focus, tx)));

            assert_eq!(ran, Some(Outcome::Ran));
            assert_eq!(
                notified.recv_timeout(WOKEN),
                Ok(()),
                "the command woke the GLib wait, with no pump after it"
            );
        },
    );
}
