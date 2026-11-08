// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::PrivateContext;

#[test]
fn pending_lookup_finishes_promptly_without_reentering() {
    let root = tempfile::tempdir().expect("fixture");
    let location = Location::local(root.path());
    let context = PrivateContext::new();
    context
        .with_thread_default(|| {
            let result = Rc::new(RefCell::new(None));
            let sink = result.clone();
            let pending = PendingVolumeLookup::start(
                &[&location],
                Box::new(move |lookup| {
                    *sink.borrow_mut() = Some(lookup);
                }),
            );
            assert!(result.borrow().is_none(), "callback must not be reentrant");
            let started = Instant::now();
            while result.borrow().is_none() && started.elapsed() < REMOTE_QUERY_TIMEOUT / 2 {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                result.borrow().is_some(),
                "completion must not wait for the timeout"
            );
            assert_eq!(
                Rc::strong_count(&pending.state),
                1,
                "a finished lookup must not leave its timeout task holding the state"
            );
        })
        .expect("private context");
}

#[test]
fn cancelled_lookup_leaves_no_timeout_task_holding_its_state() {
    let root = tempfile::tempdir().expect("fixture");
    let location = Location::local(root.path());
    let context = PrivateContext::new();
    context
        .with_thread_default(|| {
            let pending = PendingVolumeLookup::start(&[&location], Box::new(|_| {}));
            let state = pending.state.clone();
            drop(pending);
            let started = Instant::now();
            while Rc::strong_count(&state) > 1 && started.elapsed() < REMOTE_QUERY_TIMEOUT / 2 {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(Rc::strong_count(&state), 1);
        })
        .expect("private context");
}
