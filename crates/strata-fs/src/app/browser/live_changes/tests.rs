// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT;

const BURST: usize = 2_000;

struct Watched {
    browser: Rc<Browser>,
    events: Rc<RefCell<Vec<BrowserEvent>>>,
    notify: WatchCallback,
}

fn watched_browser() -> Watched {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();
    let notify = notify
        .borrow()
        .clone()
        .expect("installed directory watcher");
    Watched {
        browser,
        events,
        notify,
    }
}

/// The monitor reports `BURST` new entries, then the removal of the first.
fn report_burst(watched: &Watched) {
    for index in 0..BURST {
        (watched.notify)(DirectoryChange::Upsert(batch_entry(&format!(
            "new-{index:04}"
        ))));
    }
    (watched.notify)(DirectoryChange::Remove(batch_entry("new-0000").location));
}

fn burst_result() -> Vec<String> {
    std::iter::once("child".to_owned())
        .chain((1..BURST).map(|index| format!("new-{index:04}")))
        .collect()
}

fn splice_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
        .count()
}

fn settle(browser: &Rc<Browser>) {
    pump_until(|| !browser.live_changes.batching.get());
}

#[test]
fn a_monitor_burst_lands_in_order_through_a_few_batches() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let watched = watched_browser();
    let browser = &watched.browser;

    report_burst(&watched);
    assert!(column_names(browser, 0).len() < BURST);
    settle(browser);

    assert_eq!(column_names(browser, 0), burst_result());
    assert!(
        splice_count(&watched.events) < BURST / 2,
        "{} splices for {} changes",
        splice_count(&watched.events),
        BURST + 1
    );
}

#[test]
fn a_starting_file_operation_publishes_the_burst_queued_before_it() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let watched = watched_browser();
    let browser = &watched.browser;
    let root = Location::local("/fixture");
    report_burst(&watched);
    assert!(column_names(browser, 0).len() < BURST);

    let request_id = browser.begin_operation();
    browser.deletion_operation.set(true);
    assert_eq!(column_names(browser, 0), burst_result());
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    let removed = batch_entry("new-0001").location;
    browser.handle_directory_change(0, &root, DirectoryChange::Remove(removed));
    settle(browser);
    assert_eq!(column_names(browser, 0), burst_result());
    callback(OperationEvent::Deleted {
        request_id,
        locations: Vec::new(),
    });

    let mut remaining = burst_result();
    remaining.retain(|name| name != "new-0001");
    assert_eq!(column_names(browser, 0), remaining);
}
