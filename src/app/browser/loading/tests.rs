// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use super::*;
use crate::{
    model::{EntryKind, Location, MetadataValue, ViewPreferences},
    services::{DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};

#[derive(Default)]
struct PendingSource {
    last_request: Cell<Option<RequestId>>,
}

impl FileSource for PendingSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, _: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.last_request.set(Some(request.id));
        LoadHandle::new(|| {})
    }
}

pub(super) struct Fixture {
    pub(super) browser: Rc<Browser>,
    pub(super) events: Rc<RefCell<Vec<BrowserEvent>>>,
    pub(super) location: Location,
    source: Rc<PendingSource>,
}

impl Fixture {
    pub(super) fn new(location: Location) -> Self {
        let source = Rc::new(PendingSource::default());
        let browser = Browser::new(source.clone());
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        browser.navigate(location.clone());
        events.borrow_mut().clear();
        Self {
            browser,
            events,
            location,
            source,
        }
    }

    pub(super) fn request(&self) -> RequestId {
        self.browser
            .state
            .borrow()
            .request_id_for_depth(0)
            .expect("directory request")
    }

    pub(super) fn entry(&self, name: &str) -> FileEntry {
        FileEntry {
            location: self
                .location
                .child(std::ffi::OsStr::new(name))
                .expect("fixture child"),
            native_name: name.into(),
            display_name: name.into(),
            thumbnail_path: None,
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            is_hidden: name.starts_with('.'),
        }
    }

    pub(super) fn batch(&self, entries: Vec<FileEntry>) {
        self.browser.handle_directory_event(DirectoryEvent::Batch {
            request_id: self.request(),
            entries,
        });
    }

    pub(super) fn finish(&self) {
        self.browser
            .handle_directory_event(DirectoryEvent::Finished {
                request_id: self.request(),
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.browser.clear_observer();
        self.browser.cancel_deferred_work();
    }
}

fn roots() -> [Location; 2] {
    [
        Location::local("/fixture"),
        Location::uri("sftp://example.test/fixture"),
    ]
}

#[test]
fn native_and_remote_completions_preserve_capabilities_and_terminal_order() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for location in roots() {
        for (truncated, can_trash, can_delete) in [
            (true, Some(false), Some(true)),
            (false, Some(true), None),
            (false, None, Some(false)),
        ] {
            let fixture = Fixture::new(location.clone());
            fixture.batch(vec![fixture.entry("entry")]);
            fixture
                .browser
                .handle_directory_event(DirectoryEvent::Finished {
                    request_id: fixture.request(),
                    truncated,
                    can_trash,
                    can_delete,
                });
            let column = fixture.browser.column_snapshot(0).expect("loaded column");
            assert_eq!(column.count, 1);
            assert!(!column.loading);
            assert_eq!(column.truncated, truncated);
            assert_eq!(fixture.browser.can_trash_at(0), can_trash);
            assert_eq!(fixture.browser.can_delete_at(0), can_delete);
            let events = fixture.events.borrow();
            assert!(
                matches!(events.last(), Some(BrowserEvent::LoadFinished { depth: 0, truncated: actual }) if *actual == truncated)
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn stale_loading_events_cannot_mutate_a_replacement_request() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for location in roots() {
        let fixture = Fixture::new(location);
        let old = fixture.request();
        fixture.batch(vec![fixture.entry("discarded")]);
        fixture.browser.navigate(Location::local("/replacement"));
        fixture.events.borrow_mut().clear();
        for event in [
            DirectoryEvent::Batch {
                request_id: old,
                entries: vec![fixture.entry("late")],
            },
            DirectoryEvent::MetadataIncomplete { request_id: old },
            DirectoryEvent::Finished {
                request_id: old,
                truncated: true,
                can_trash: Some(false),
                can_delete: Some(false),
            },
            DirectoryEvent::Failed {
                request_id: old,
                message: "late failure".into(),
            },
        ] {
            fixture.browser.handle_directory_event(event);
        }
        let column = fixture
            .browser
            .column_snapshot(0)
            .expect("replacement column");
        assert!(column.loading);
        assert_eq!(column.count, 0);
        assert!(column.error.is_none());
        assert!(fixture.events.borrow().is_empty());
        assert!(fixture.browser.staging.borrow().is_empty());
        assert!(fixture.browser.remote.borrow().has_no_work());
        fixture.finish();
        assert!(
            !fixture
                .browser
                .column_snapshot(0)
                .expect("finished replacement")
                .loading
        );
    }
}

#[test]
fn native_failure_discards_staged_entries_without_publishing_them() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    let fixture = Fixture::new(Location::local("/fixture"));
    fixture
        .browser
        .handle_directory_event(DirectoryEvent::MetadataIncomplete {
            request_id: fixture.request(),
        });
    fixture.batch(vec![fixture.entry("partial")]);
    assert!(
        fixture
            .browser
            .staging
            .borrow()
            .get(&0)
            .expect("staged load")
            .metadata_incomplete
    );
    fixture
        .browser
        .handle_directory_event(DirectoryEvent::Failed {
            request_id: fixture.request(),
            message: "load failed".into(),
        });
    assert!(fixture.browser.staging.borrow().is_empty());
    assert!(fixture.browser.sorting.borrow().is_empty());
    assert!(fixture.browser.staged_publishes.borrow().is_empty());
    let column = fixture.browser.column_snapshot(0).expect("failed column");
    assert_eq!(column.count, 0);
    assert_eq!(column.error.as_deref(), Some("load failed"));
    assert!(matches!(
        fixture.events.borrow().as_slice(),
        [BrowserEvent::LoadFailed { depth: 0, .. }]
    ));
}

#[test]
fn completed_requests_reject_batches_but_retain_the_existing_failure_transition() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for location in roots() {
        let fixture = Fixture::new(location);
        fixture.batch(vec![fixture.entry("accepted")]);
        fixture.finish();
        fixture.events.borrow_mut().clear();
        fixture.batch(vec![fixture.entry("late")]);
        fixture.finish();
        assert!(fixture.events.borrow().is_empty());
        assert_eq!(
            fixture
                .browser
                .column_snapshot(0)
                .expect("completed column")
                .count,
            1
        );
        // Failure applies by owning request, unlike the open-load batch/finish gate.
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::Failed {
                request_id: fixture.request(),
                message: "provider failure".into(),
            });
        assert!(matches!(
            fixture.events.borrow().as_slice(),
            [BrowserEvent::LoadFailed { depth: 0, .. }]
        ));
    }
}

#[test]
fn completion_observers_can_navigate_without_borrowing_or_replacing_new_state() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for location in roots() {
        let fixture = Fixture::new(location);
        let weak = Rc::downgrade(&fixture.browser);
        fixture.browser.observe(move |event| {
            if matches!(event, BrowserEvent::LoadFinished { .. }) {
                weak.upgrade()
                    .expect("observed browser")
                    .navigate(Location::local("/replacement"));
            }
        });
        fixture.batch(vec![fixture.entry("entry")]);
        fixture.finish();
        let column = fixture
            .browser
            .column_snapshot(0)
            .expect("replacement column");
        assert_eq!(column.location, Location::local("/replacement"));
        assert!(column.loading);
        assert_eq!(column.count, 0);
    }
}

#[test]
fn peek_events_keep_hidden_filtering_and_terminals_separate_from_columns() {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for show_hidden in [false, true] {
        let fixture = Fixture::new(Location::local("/fixture"));
        fixture.finish();
        fixture.browser.apply_default_preferences(ViewPreferences {
            show_hidden,
            ..fixture.browser.preferences()
        });
        fixture
            .browser
            .begin_peek(0, Location::local("/fixture/peek"));
        let request_id = fixture.source.last_request.get().expect("peek request");
        fixture.events.borrow_mut().clear();
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::Batch {
                request_id,
                entries: vec![fixture.entry(".hidden"), fixture.entry("visible")],
            });
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::MetadataIncomplete { request_id });
        fixture
            .browser
            .handle_directory_event(DirectoryEvent::Finished {
                request_id,
                truncated: true,
                can_trash: Some(false),
                can_delete: Some(false),
            });
        let events = fixture.events.borrow();
        assert!(
            matches!(events.as_slice(), [BrowserEvent::PeekEntriesAdded { entries }, BrowserEvent::PeekFinished] if entries.len() == if show_hidden { 2 } else { 1 })
        );
        assert!(fixture.browser.staging.borrow().is_empty());
        assert_eq!(
            fixture
                .browser
                .column_snapshot(0)
                .expect("parent column")
                .count,
            0
        );
    }
}
