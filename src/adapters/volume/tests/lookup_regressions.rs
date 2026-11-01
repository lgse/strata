// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn pending_lookup_finishes_promptly_without_reentering() {
    let root = tempfile::tempdir().expect("fixture");
    let location = Location::local(root.path());
    let context = glib::MainContext::new();
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
            drop(pending);
        })
        .expect("private context");
}
