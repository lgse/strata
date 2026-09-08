// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn presentation() -> DelayedLoading {
    let stack = gtk::Stack::new();
    for name in ["content", "loading", "feedback"] {
        stack.add_named(&gtk::Label::new(Some(name)), Some(name));
    }
    DelayedLoading::new(&stack)
}

fn source(loading: &DelayedLoading) -> glib::Source {
    glib::MainContext::default()
        .find_source_by_id(loading.pending.0.borrow().as_ref().expect("pending timer"))
        .expect("attached timer")
}

fn expire_grace_period() {
    std::thread::sleep(GRACE_PERIOD + Duration::from_millis(20));
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn delay_lifecycle() {
    crate::test_support::gtk_test(
        "ui::loading_skeleton::delay::tests::delay_lifecycle",
        || {
            let started = glib::monotonic_time();
            let loading = presentation();
            assert_eq!(
                loading.stack.visible_child_name().as_deref(),
                Some("pending")
            );
            assert!(source(&loading).ready_time() >= started + 150_000);
            expire_grace_period();
            assert_eq!(
                loading.stack.visible_child_name().as_deref(),
                Some("loading")
            );
            assert!(loading.pending.0.borrow().is_none());
            loading.show("content");

            for outcome in ["content", "feedback"] {
                loading.start();
                let timer = source(&loading);
                loading.clone().show(outcome);
                assert!(timer.is_destroyed());
                expire_grace_period();
                assert_eq!(loading.stack.visible_child_name().as_deref(), Some(outcome));
            }

            loading.start();
            let old_timer = source(&loading);
            let restarted = glib::monotonic_time();
            loading.clone().start();
            assert!(old_timer.is_destroyed());
            assert!(source(&loading).ready_time() >= restarted + 150_000);
            assert_eq!(
                loading.stack.visible_child_name().as_deref(),
                Some("pending")
            );
            expire_grace_period();
            assert_eq!(
                loading.stack.visible_child_name().as_deref(),
                Some("loading")
            );

            loading.start();
            let timer = source(&loading);
            let stack = loading.stack.clone();
            drop(loading);
            assert!(timer.is_destroyed());
            expire_grace_period();
            assert_eq!(stack.visible_child_name().as_deref(), Some("pending"));

            let loading = presentation();
            let weak_stack = loading.stack.downgrade();
            drop(loading);
            assert!(weak_stack.upgrade().is_none());
        },
    );
}
