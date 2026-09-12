// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use super::*;
use crate::{
    model::{EntryKind, Location, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};

struct PendingSource;

impl FileSource for PendingSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }
    fn enumerate(&self, _: DirectoryRequest, _: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

fn entry(index: usize) -> FileEntry {
    let name = format!("entry-{index}");
    FileEntry {
        location: Location::uri(format!("sftp://example.test/{name}")),
        native_name: name.clone().into(),
        display_name: name,
        thumbnail_path: None,
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
    }
}

fn fixture() -> (Rc<Browser>, Rc<RefCell<Vec<BrowserEvent>>>, RequestId) {
    let browser = Browser::new(Rc::new(PendingSource));
    browser.navigate(Location::uri("sftp://example.test/root"));
    let request_id = browser
        .state
        .borrow()
        .request_id_for_depth(0)
        .expect("remote request");
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    (browser, events, request_id)
}

fn terminal_count(events: &[BrowserEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                BrowserEvent::LoadFinished { .. } | BrowserEvent::LoadFailed { .. }
            )
        })
        .count()
}

fn inserted_rows(events: &[BrowserEvent]) -> usize {
    events
        .iter()
        .map(|event| match event {
            BrowserEvent::EntriesInserted { insertions, .. } => insertions
                .iter()
                .map(|insertion| insertion.entries.len())
                .sum(),
            _ => 0,
        })
        .sum()
}

#[test]
fn depths_share_one_coalescing_timer() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, _, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(1)],
    });
    browser.descend(0, Location::uri("sftp://example.test/root/child"));
    let child = browser
        .state
        .borrow()
        .request_id_for_depth(1)
        .expect("child request");
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: child,
        entries: vec![entry(2)],
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: child,
        entries: vec![entry(3)],
    });
    assert!(browser.remote.borrow().has_pending(0));
    assert!(browser.remote.borrow().has_pending(1));
    let source = browser
        .remote
        .borrow()
        .flush_timer
        .as_ref()
        .expect("shared timer")
        .as_raw();
    browser.flush_coalesced_capped(Some(0));
    assert!(browser.remote.borrow().has_pending(1));
    assert_eq!(
        browser
            .remote
            .borrow()
            .flush_timer
            .as_ref()
            .expect("child timer")
            .as_raw(),
        source
    );
    browser.flush_coalesced_capped(Some(1));
    assert!(!browser.remote.borrow().timer_armed());
    assert!(browser.remote.borrow().has_no_work());
    assert_eq!(browser.column_snapshot(0).expect("root").count, 2);
    assert_eq!(browser.column_snapshot(1).expect("child").count, 2);
}

#[test]
fn superseding_a_load_discards_queued_batches_and_armed_timer() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(1)],
    });
    assert!(
        browser
            .remote
            .borrow()
            .flush_timer
            .as_ref()
            .expect("old timer")
            .as_raw()
            > 0
    );
    events.borrow_mut().clear();
    browser.navigate(Location::local("/replacement"));
    assert!(!browser.remote.borrow().timer_armed());
    events.borrow_mut().clear();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(2)],
    });
    browser.handle_directory_event(DirectoryEvent::Finished {
        request_id: request,
        truncated: true,
        can_trash: None,
        can_delete: None,
    });
    browser.flush_coalesced_capped(None);
    assert!(!browser.remote.borrow().timer_armed());
    assert!(browser.state.borrow().request_id_for_depth(0).is_some());
    assert!(browser.remote.borrow().has_no_work());
    assert!(events.borrow().is_empty());
}

#[test]
fn capped_drains_preserve_all_entries_and_deferred_terminals() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    events.borrow_mut().clear();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: (1..=1536).map(entry).collect(),
    });
    browser.handle_directory_event(DirectoryEvent::Finished {
        request_id: request,
        truncated: true,
        can_trash: Some(false),
        can_delete: Some(true),
    });
    assert_eq!(inserted_rows(&events.borrow()), 512);
    assert!(browser.column_snapshot(0).expect("loading column").loading);
    assert_eq!(terminal_count(&events.borrow()), 0);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(inserted_rows(&events.borrow()), 1024);
    assert!(
        browser
            .column_snapshot(0)
            .expect("still loading column")
            .loading
    );
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(
        browser.column_snapshot(0).expect("after final drain").count,
        1537
    );
    assert!(!browser.column_snapshot(0).expect("finished column").loading);
    assert_eq!(terminal_count(&events.borrow()), 1);
    assert!(matches!(
        events.borrow().last(),
        Some(BrowserEvent::LoadFinished {
            depth: 0,
            truncated: true
        })
    ));
    let rows = browser.column_snapshot(0).expect("rows").count;
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(browser.column_snapshot(0).expect("stable rows").count, rows);
    assert_eq!(terminal_count(&events.borrow()), 1);
    assert!(!browser.remote.borrow().timer_armed());
    assert!(browser.remote.borrow().has_no_work());
}

#[test]
fn capped_drains_defer_failure_until_the_queue_is_empty() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    events.borrow_mut().clear();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: (1..=1025).map(entry).collect(),
    });
    browser.handle_directory_event(DirectoryEvent::Failed {
        request_id: request,
        message: "deferred failure".into(),
    });
    assert_eq!(inserted_rows(&events.borrow()), 512);
    assert!(browser.column_snapshot(0).expect("still loading").loading);
    assert_eq!(terminal_count(&events.borrow()), 0);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(inserted_rows(&events.borrow()), 1024);
    assert!(browser.column_snapshot(0).expect("still loading").loading);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(browser.column_snapshot(0).expect("final row").count, 1026);
    assert!(!browser.column_snapshot(0).expect("failed column").loading);
    assert!(
        matches!(events.borrow().last(), Some(BrowserEvent::LoadFailed { depth: 0, message }) if message == "deferred failure")
    );
    assert_eq!(terminal_count(&events.borrow()), 1);
    assert!(!browser.remote.borrow().timer_armed());
    assert!(browser.remote.borrow().has_no_work());
}

#[test]
fn cleanup_then_late_flush_cannot_publish_or_finish_old_work() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    events.borrow_mut().clear();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: (1..=1025).map(entry).collect(),
    });
    browser.handle_directory_event(DirectoryEvent::Finished {
        request_id: request,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    assert!(browser.remote.borrow().timer_armed());
    assert!(browser.remote.borrow().has_pending(0));
    browser.navigate(Location::local("/elsewhere"));
    assert!(!browser.remote.borrow().timer_armed());
    events.borrow_mut().clear();
    browser.flush_coalesced_capped(None);
    assert!(browser.remote.borrow().has_no_work());
    assert!(events.borrow().is_empty());

    let (browser, events, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    browser.descend(0, Location::uri("sftp://example.test/root/child"));
    let removed = browser
        .state
        .borrow()
        .request_id_for_depth(1)
        .expect("child request");
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: removed,
        entries: vec![entry(10)],
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: removed,
        entries: (11..=523).map(entry).collect(),
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: (1..=513).map(entry).collect(),
    });
    browser.descend(0, Location::uri("sftp://example.test/root/other-child"));
    events.borrow_mut().clear();
    browser.flush_coalesced_capped(None);
    assert_eq!(
        browser.column_snapshot(0).expect("retained root").count,
        513
    );
    assert!(browser.remote.borrow().has_pending(0));
    assert!(browser.remote.borrow().timer_armed());
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesInserted { depth: 1, .. }
            | BrowserEvent::EntriesReplaced { depth: 1, .. }
    )));

    browser.flush_coalesced_capped(None);
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("fully drained root")
            .count,
        514
    );
    assert!(!browser.remote.borrow().has_pending(0));
    assert!(!browser.remote.borrow().timer_armed());
    assert!(browser.remote.borrow().has_no_work());
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesInserted { depth: 1, .. }
            | BrowserEvent::EntriesReplaced { depth: 1, .. }
    )));
}

#[test]
fn observer_can_reenter_queue_and_cleanup_without_refcell_panic() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(0)],
    });
    let weak: Weak<Browser> = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(
            event,
            BrowserEvent::EntriesInserted { .. } | BrowserEvent::EntriesReplaced { .. }
        ) && let Some(browser) = weak.upgrade()
        {
            browser.accumulate_batch(request, 0, vec![entry(9_999)]);
            browser.remote.borrow_mut().clear();
        }
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: vec![entry(1)],
    });
    browser.flush_coalesced_capped(Some(0));
    assert!(browser.remote.borrow().has_no_work());
    assert!(!browser.remote.borrow().timer_armed());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesReplaced { depth: 0, .. }
            | BrowserEvent::EntriesInserted { depth: 0, .. }
    )));
    assert!(browser.remote.borrow().has_no_work());
    assert!(!browser.remote.borrow().timer_armed());
}
