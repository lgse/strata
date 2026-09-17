// SPDX-License-Identifier: MIT

use super::*;
use crate::app::browser::{
    loading::LoadCompletion,
    sorting::{SortPlan, SortTask},
};

fn task(request_id: RequestId) -> SortTask {
    SortTask {
        depth: 0,
        request_id,
        plan: SortPlan {
            ordering_preferences: ViewPreferences::default(),
            staged_preferences: ViewPreferences::default(),
            retry_metadata: false,
            completion: LoadCompletion {
                truncated: true,
                can_trash: Some(false),
                can_delete: Some(true),
            },
        },
    }
}

fn install_sort(browser: &Browser, request_id: RequestId) {
    browser
        .state
        .borrow_mut()
        .navigate(Location::local("/fixture"), request_id);
    browser.sorting.borrow_mut().insert(
        0,
        SortingLoad {
            request_id,
            deltas: Vec::new(),
        },
    );
}

#[test]
fn stale_worker_completion_preserves_replacement_and_its_monitor_deltas() {
    for failed in [false, true] {
        let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
        install_sort(&browser, RequestId(2));
        let watched = Location::local("/fixture");
        browser.handle_directory_change(
            0,
            &watched,
            DirectoryChange::Remove(batch_entry("removed").location),
        );
        browser.handle_directory_change(0, &watched, DirectoryChange::Upsert(batch_entry("added")));

        if failed {
            browser.fail_staged_sort(task(RequestId(1)));
        } else {
            browser.finish_staged_sort(task(RequestId(1)), vec![batch_entry("stale")]);
        }
        assert!(
            events.borrow().is_empty(),
            "stale completion must be silent"
        );
        assert!(
            browser
                .column_snapshot(0)
                .expect("replacement column")
                .loading
        );

        browser.finish_staged_sort(
            task(RequestId(2)),
            vec![batch_entry("kept"), batch_entry("removed")],
        );
        assert_eq!(column_names(&browser, 0), ["added", "kept"]);
        let snapshot = browser
            .column_snapshot(0)
            .expect("completed replacement column");
        assert!(!snapshot.loading);
        assert!(snapshot.truncated);
        let events = events.borrow();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(BrowserEvent::LoadFinished {
                truncated: true,
                ..
            })
        ));
    }
}

#[test]
fn failed_worker_finishes_its_load_once() {
    let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    install_sort(&browser, RequestId(1));
    browser.fail_staged_sort(task(RequestId(1)));
    browser.fail_staged_sort(task(RequestId(1)));
    let snapshot = browser.column_snapshot(0).expect("failed column");
    assert!(!snapshot.loading);
    assert_eq!(
        snapshot.error.as_deref(),
        Some("Sorting the directory failed.")
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, BrowserEvent::LoadFailed { .. }))
            .count(),
        1
    );
}

#[test]
fn completed_worker_cannot_publish_into_a_removed_column() {
    let (browser, events, _) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    install_sort(&browser, RequestId(1));
    browser.state.borrow_mut().columns.clear();
    browser.finish_staged_sort(task(RequestId(1)), vec![batch_entry("orphan")]);
    assert!(browser.column_snapshot(0).is_none());
    assert!(events.borrow().is_empty());
}

#[test]
fn background_sort_publishes_sorted_rows_before_its_terminal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: (0..2050)
            .rev()
            .map(|index| batch_entry(&format!("file-{index:04}")))
            .collect(),
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    assert!(browser.column_snapshot(0).expect("sorting column").loading);
    pump_until(|| {
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    });
    let expected: Vec<_> = (0..2050).map(|index| format!("file-{index:04}")).collect();
    assert_eq!(column_names(&browser, 0), expected);
    assert!(matches!(
        events.borrow().last(),
        Some(BrowserEvent::LoadFinished { .. })
    ));
    assert!(!browser.column_snapshot(0).expect("sorted column").loading);
}
