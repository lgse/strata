// SPDX-License-Identifier: MIT

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use super::GlibLoop;

const PATIENCE: Duration = Duration::from_secs(10);
const BUDGET: Duration = Duration::from_millis(8);
const QUIET: Duration = Duration::from_millis(200);

/// A context of the test's own, owned by its thread, with its loop and the helper's
/// notifications. Private contexts let the tests run side by side.
struct Owned {
    glib: GlibLoop,
    context: glib::MainContext,
    notified: Receiver<()>,
    /// How many times a source of the test ran.
    ran: Arc<AtomicU32>,
}

impl Owned {
    fn ran(&self) -> u32 {
        self.ran.load(Ordering::SeqCst)
    }

    /// A callback for a source: it counts a run, and runs `times` in all.
    fn counting(&self, times: u32) -> impl FnMut() -> glib::ControlFlow + Send + 'static {
        let ran = self.ran.clone();
        move || {
            if ran.fetch_add(1, Ordering::SeqCst) + 1 < times {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        }
    }

    fn wait_for_a_notification(&self) {
        self.notified
            .recv_timeout(PATIENCE)
            .expect("the helper notifies");
    }
}

fn owned(run: impl FnOnce(&Owned)) {
    let context = glib::MainContext::new();
    let _owner = context.acquire().expect("a new context has no owner");
    let (notify, notified) = mpsc::channel();
    let glib = GlibLoop::start(context.clone(), move || {
        let _ = notify.send(());
    })
    .expect("the loop starts");
    run(&Owned {
        glib,
        context: context.clone(),
        notified,
        ran: Arc::default(),
    });
}

/// GLib tells a context owned elsewhere by writing to its wake-up descriptor, so this is also
/// the helper waking for a descriptor.
#[test]
fn a_source_from_another_thread_notifies_and_the_pump_dispatches_it() {
    owned(|o| {
        assert!(!o.glib.pump(BUDGET));
        let (context, run) = (o.context.clone(), o.counting(1));
        std::thread::spawn(move || {
            let mut run = run;
            context.invoke(move || {
                run();
            });
        });

        o.wait_for_a_notification();

        assert!(o.glib.pump(BUDGET));
        assert_eq!(o.ran(), 1);
    });
}

#[test]
fn a_timeout_attached_on_the_owning_thread_is_waited_for_after_the_next_pump() {
    owned(|o| {
        o.glib.pump(BUDGET);
        glib::timeout_source_new(
            Duration::from_millis(30),
            None,
            glib::Priority::DEFAULT,
            o.counting(1),
        )
        .attach(Some(&o.context));
        assert!(!o.glib.pump(BUDGET), "the timeout is not due yet");

        o.wait_for_a_notification();

        assert!(o.glib.pump(BUDGET));
        assert_eq!(o.ran(), 1);
    });
}

#[test]
fn an_idle_context_sends_no_notification_however_often_it_is_pumped() {
    owned(|o| {
        for _ in 0..3 {
            assert!(!o.glib.pump(BUDGET));
        }

        assert_eq!(
            o.notified.recv_timeout(QUIET),
            Err(RecvTimeoutError::Timeout)
        );
    });
}

#[test]
fn a_pass_that_runs_out_of_budget_notifies_again_at_once() {
    owned(|o| {
        glib::idle_source_new(None, glib::Priority::DEFAULT, o.counting(3))
            .attach(Some(&o.context));

        for pass in 1..=3 {
            assert!(o.glib.pump(Duration::ZERO));
            assert_eq!(o.ran(), pass, "one dispatch a pass");
            if pass < 3 {
                o.wait_for_a_notification();
            }
        }
    });
}

#[test]
fn a_thread_that_does_not_own_the_context_cannot_start_the_loop() {
    let unowned = glib::MainContext::new();

    assert!(GlibLoop::start(unowned, || {}).is_err());
}
