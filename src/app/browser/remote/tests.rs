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
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
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

fn batch(browser: &Rc<Browser>, request_id: RequestId, indices: impl IntoIterator<Item = usize>) {
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id,
        entries: indices.into_iter().map(entry).collect(),
    });
}

#[track_caller]
fn row_count(browser: &Browser, depth: usize) -> usize {
    browser
        .column_snapshot(depth)
        .expect("column snapshot")
        .count
}

#[track_caller]
fn timer_id(browser: &Browser) -> u32 {
    browser
        .remote
        .borrow()
        .flush_timer
        .as_ref()
        .expect("armed timer")
        .as_raw()
}

#[track_caller]
fn assert_loading_progress(
    browser: &Browser,
    events: &RefCell<Vec<BrowserEvent>>,
    inserted: usize,
) {
    assert_eq!(inserted_rows(&events.borrow()), inserted);
    assert!(browser.column_snapshot(0).expect("loading column").loading);
    assert_eq!(terminal_count(&events.borrow()), 0);
}

#[track_caller]
fn assert_idle(browser: &Browser) {
    assert!(!browser.remote.borrow().timer_armed());
    assert!(browser.remote.borrow().has_no_work());
}

#[track_caller]
fn assert_no_rows_at(events: &RefCell<Vec<BrowserEvent>>, removed_depth: usize) {
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesInserted { depth, .. }
            | BrowserEvent::EntriesReplaced { depth, .. } if *depth == removed_depth
    )));
}

#[test]
fn camera_backlogs_yield_to_input_and_publish_small_chunks() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let context = gio::glib::MainContext::default();
    let _owner = context.acquire().expect("main context");
    let browser = Browser::new(Rc::new(PendingSource));
    browser.navigate(Location::uri("gphoto2://camera/"));
    let request = browser
        .state
        .borrow()
        .request_id_for_depth(0)
        .expect("request");
    let camera_entry = |index| {
        let mut entry = entry(index);
        entry.location = Location::uri(format!("gphoto2://camera/202606/photo-{index}.jpg"));
        entry
    };
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: (0..128).map(camera_entry).collect(),
    });
    browser.handle_directory_event(DirectoryEvent::Batch {
        request_id: request,
        entries: (128..128 + super::super::COALESCE_ENTRIES)
            .map(camera_entry)
            .collect(),
    });
    assert_eq!(
        row_count(&browser, 0),
        super::super::CAMERA_FLUSH_CAP,
        "neither the first batch nor a full queue may monopolize the producer callback"
    );
    let input = Rc::new(std::cell::Cell::new(false));
    let fired = input.clone();
    gio::glib::idle_add_local_full(gio::glib::Priority::DEFAULT, move || {
        fired.set(true);
        gio::glib::ControlFlow::Break
    });
    // Make both sources ready without dispatching either, then exercise priority ordering.
    std::thread::sleep(super::super::REMOTE_FLUSH_DELAY + std::time::Duration::from_millis(5));
    context.iteration(false);
    assert!(
        input.get(),
        "input-priority work must run ahead of a ready camera flush"
    );
    assert_eq!(row_count(&browser, 0), super::super::CAMERA_FLUSH_CAP);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(row_count(&browser, 0), 2 * super::super::CAMERA_FLUSH_CAP);
    browser.cancel_remote_timer();
}

#[test]
fn depths_share_one_coalescing_timer() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, _, request) = fixture();
    batch(&browser, request, [0]);
    batch(&browser, request, [1]);
    browser.descend(0, Location::uri("sftp://example.test/root/child"));
    let child = browser
        .state
        .borrow()
        .request_id_for_depth(1)
        .expect("child request");
    batch(&browser, child, [2]);
    batch(&browser, child, [3]);
    assert!(browser.remote.borrow().has_pending(0));
    assert!(browser.remote.borrow().has_pending(1));
    let source = timer_id(&browser);
    browser.flush_coalesced_capped(Some(0));
    assert!(browser.remote.borrow().has_pending(1));
    assert_eq!(timer_id(&browser), source);
    browser.flush_coalesced_capped(Some(1));
    assert_idle(&browser);
    assert_eq!(row_count(&browser, 0), 2);
    assert_eq!(row_count(&browser, 1), 2);
}

#[test]
fn superseding_a_load_discards_queued_batches_and_armed_timer() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    batch(&browser, request, [0]);
    batch(&browser, request, [1]);
    assert!(timer_id(&browser) > 0);
    events.borrow_mut().clear();
    browser.navigate(Location::local("/replacement"));
    assert!(!browser.remote.borrow().timer_armed());
    events.borrow_mut().clear();
    batch(&browser, request, [2]);
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
    batch(&browser, request, [0]);
    events.borrow_mut().clear();
    batch(&browser, request, 1..=1536);
    browser.handle_directory_event(DirectoryEvent::Finished {
        request_id: request,
        truncated: true,
        can_trash: Some(false),
        can_delete: Some(true),
    });
    assert_loading_progress(&browser, &events, 512);
    browser.flush_coalesced_capped(Some(0));
    assert_loading_progress(&browser, &events, 1024);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(row_count(&browser, 0), 1537);
    assert!(!browser.column_snapshot(0).expect("finished column").loading);
    assert_eq!(terminal_count(&events.borrow()), 1);
    assert!(matches!(
        events.borrow().last(),
        Some(BrowserEvent::LoadFinished {
            depth: 0,
            truncated: true
        })
    ));
    let rows = row_count(&browser, 0);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(row_count(&browser, 0), rows);
    assert_eq!(terminal_count(&events.borrow()), 1);
    assert_idle(&browser);
}

#[test]
fn capped_drains_defer_failure_until_the_queue_is_empty() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    batch(&browser, request, [0]);
    events.borrow_mut().clear();
    batch(&browser, request, 1..=1025);
    browser.handle_directory_event(DirectoryEvent::Failed {
        request_id: request,
        message: "deferred failure".into(),
    });
    assert_loading_progress(&browser, &events, 512);
    browser.flush_coalesced_capped(Some(0));
    assert_loading_progress(&browser, &events, 1024);
    browser.flush_coalesced_capped(Some(0));
    assert_eq!(row_count(&browser, 0), 1026);
    assert!(!browser.column_snapshot(0).expect("failed column").loading);
    assert!(
        matches!(events.borrow().last(), Some(BrowserEvent::LoadFailed { depth: 0, message }) if message == "deferred failure")
    );
    assert_eq!(terminal_count(&events.borrow()), 1);
    assert_idle(&browser);
}

#[test]
fn cleanup_then_late_flush_cannot_publish_or_finish_old_work() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    batch(&browser, request, [0]);
    events.borrow_mut().clear();
    batch(&browser, request, 1..=1025);
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
}

#[test]
fn replacing_a_child_preserves_parent_backlog_without_publishing_child_rows() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    batch(&browser, request, [0]);
    browser.descend(0, Location::uri("sftp://example.test/root/child"));
    let removed = browser
        .state
        .borrow()
        .request_id_for_depth(1)
        .expect("child request");
    batch(&browser, removed, [10]);
    batch(&browser, removed, 11..=523);
    batch(&browser, request, 1..=513);
    browser.descend(0, Location::uri("sftp://example.test/root/other-child"));
    events.borrow_mut().clear();
    browser.flush_coalesced_capped(None);
    assert_eq!(row_count(&browser, 0), 513);
    assert!(browser.remote.borrow().has_pending(0));
    assert!(browser.remote.borrow().timer_armed());
    assert_no_rows_at(&events, 1);

    browser.flush_coalesced_capped(None);
    assert_eq!(row_count(&browser, 0), 514);
    assert!(!browser.remote.borrow().has_pending(0));
    assert_idle(&browser);
    assert_no_rows_at(&events, 1);
}

#[test]
fn observer_can_reenter_queue_and_cleanup_without_refcell_panic() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let (browser, events, request) = fixture();
    batch(&browser, request, [0]);
    let reentered = Rc::new(std::cell::Cell::new(false));
    let observed_reentry = reentered.clone();
    let weak: Weak<Browser> = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(
            event,
            BrowserEvent::EntriesInserted { .. } | BrowserEvent::EntriesReplaced { .. }
        ) && let Some(browser) = weak.upgrade()
        {
            observed_reentry.set(true);
            browser.accumulate_batch(request, 0, vec![entry(9_999)]);
            browser.remote.borrow_mut().clear();
        }
    });
    batch(&browser, request, [1]);
    browser.flush_coalesced_capped(Some(0));
    assert!(
        reentered.get(),
        "observer must reenter the queue before cleanup"
    );
    assert_idle(&browser);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesReplaced { depth: 0, .. }
            | BrowserEvent::EntriesInserted { depth: 0, .. }
    )));
}
