// SPDX-License-Identifier: MIT
use super::*;

fn result(query: &str) -> SearchEvent {
    SearchEvent::Results {
        query: query.into(),
        items: vec![],
        indexing: false,
        coverage: SearchCoverage::default(),
        has_more: false,
    }
}

#[test]
fn draining_rejects_stale_queries_and_keeps_final_event_on_disconnect() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(result("current")).expect("send current result");
    sender.send(result("stale")).expect("send stale result");
    drop(sender);
    let (batch, disconnected) = drain(&receiver, "current");
    assert_eq!(batch.expect("current result").query, "current");
    assert!(disconnected);
}

#[test]
fn draining_is_bounded_and_dismissed_query_never_publishes() {
    let (sender, receiver) = std::sync::mpsc::channel();
    for _ in 0..9 {
        sender.send(result("current")).expect("send current result");
    }
    assert!(drain(&receiver, "current").0.is_some());
    assert!(drain(&receiver, "current").0.is_some());
    sender.send(result("")).expect("send empty result");
    assert!(drain(&receiver, "").0.is_none());
}

#[test]
fn restart_intent_cancellation_and_drop_retire_worker_delivery() {
    crate::test_support::gtk_test(
        "ui::search_session::tests::restart_intent_cancellation_and_drop_retire_worker_delivery",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::write(fixture.path().join("alpha.txt"), "a").expect("alpha file");
            std::fs::write(fixture.path().join("beta.txt"), "b").expect("beta file");
            let input = SearchInput {
                root: fixture.path().into(),
                show_hidden: false,
                recursive: true,
            };
            let session = SearchSession::default();
            let delivered = Rc::new(RefCell::new(Vec::new()));
            let output = delivered.clone();
            session.update(
                input.clone(),
                "alpha",
                false,
                Rc::new(move |batch| output.borrow_mut().push(batch.query)),
            );
            session.expect_query("beta");
            let output = delivered.clone();
            session.update(
                input.clone(),
                "beta",
                true,
                Rc::new(move |batch| output.borrow_mut().push(batch.query)),
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while delivered.borrow().is_empty() {
                assert!(std::time::Instant::now() < deadline);
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(delivered.borrow().iter().all(|query| query == "beta"));
            session.cancel();
            assert!(!session.is_active());
            assert!(session.0.source.borrow().is_none());
            let output = delivered.clone();
            session.update(
                input,
                "alpha",
                false,
                Rc::new(move |batch| output.borrow_mut().push(batch.query)),
            );
            let weak = Rc::downgrade(&session.0);
            drop(session);
            assert!(weak.upgrade().is_none(), "polling cannot own its session");
            while glib::MainContext::default().iteration(false) {}
            assert!(delivered.borrow().iter().all(|query| query == "beta"));
        },
    );
}
