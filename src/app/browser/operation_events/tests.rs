// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn stale_progress_and_terminals_cannot_contaminate_a_replacement_transfer() {
    let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let stale_id = browser.begin_operation();
    let stale = browser.operation_callback(stale_id, false, HashSet::new());
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/destination")));
    let current = browser.operation_callback(request_id, false, HashSet::new());
    events.borrow_mut().clear();
    let stale_progress = OperationEvent::TransferProgress {
        request_id: stale_id,
        completed_items: 1,
        transferred_bytes: 10,
        total_bytes: Some(10),
        created_location: Some(Location::local("/fixture/destination/stale")),
    };
    stale(stale_progress.clone());
    current(stale_progress);
    stale(OperationEvent::Pasted {
        request_id: stale_id,
        locations: Vec::new(),
    });
    stale(OperationEvent::Pasted {
        request_id,
        locations: Vec::new(),
    });
    assert!(events.borrow().is_empty());

    let created = Location::local("/fixture/destination/current");
    current(OperationEvent::TransferProgress {
        request_id,
        completed_items: 1,
        transferred_bytes: 20,
        total_bytes: Some(20),
        created_location: Some(created.clone()),
    });
    current(OperationEvent::Pasted {
        request_id,
        locations: Vec::new(),
    });
    current(OperationEvent::Pasted {
        request_id,
        locations: Vec::new(),
    });
    let events = events.borrow();
    let reveals: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::TransferReveal { locations, .. } => Some(locations.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reveals, vec![vec![created]]);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BrowserEvent::TransferCompleted))
            .count(),
        1
    );
}

#[test]
fn progress_observer_can_supersede_an_operation_before_its_terminal() {
    let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    let request_id = browser.begin_operation();
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    let replacement = Rc::new(Cell::new(None));
    let observed = replacement.clone();
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::DeletionProgress { .. }) {
            observed.set(Some(
                weak.upgrade().expect("live browser").begin_operation(),
            ));
        }
    });
    callback(OperationEvent::DeleteProgress {
        request_id,
        completed: 1,
        total: 2,
        deleted_location: None,
    });
    callback(OperationEvent::Failed {
        request_id,
        message: "stale failure".to_owned(),
    });
    let replacement_id = replacement.get().expect("replacement operation");
    browser.operation_callback(replacement_id, false, HashSet::new())(OperationEvent::Failed {
        request_id: replacement_id,
        message: "current failure".to_owned(),
    });
    let messages: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::OperationFailed { message } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(messages, ["current failure"]);
}

#[test]
fn transfer_finish_observer_navigation_suppresses_reveal_but_not_completion() {
    let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser
        .transfer_destination
        .replace(Some(Location::local("/fixture/destination")));
    browser
        .created_locations
        .replace(vec![Location::local("/fixture/destination/copied")]);
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::TransferFinished { .. }) {
            let browser = weak.upgrade().expect("live browser");
            assert!(!browser.is_current_operation(request_id));
            browser.navigate(Location::local("/elsewhere"));
        }
    });
    callback(OperationEvent::Pasted {
        request_id,
        locations: Vec::new(),
    });
    assert_eq!(
        browser.active_location(),
        Some(Location::local("/elsewhere"))
    );
    let events = events.borrow();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, BrowserEvent::TransferReveal { .. }))
    );
    assert!(matches!(
        events.last(),
        Some(BrowserEvent::TransferCompleted)
    ));
}

#[test]
fn held_operation_callback_does_not_keep_the_browser_alive() {
    let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    let request_id = browser.begin_operation();
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    let weak = Rc::downgrade(&browser);
    drop(browser);
    assert!(weak.upgrade().is_none());
    callback(OperationEvent::Failed {
        request_id,
        message: "late failure".to_owned(),
    });
    assert!(events.borrow().is_empty());
}
