// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, ffi::OsString};

use super::*;
use crate::{
    model::{EntryKind, MetadataValue},
    services::{
        CancelledOperation, CompressRequest, ExtractRequest, LoadHandle, MetadataOutcome,
        MetadataRequest, MetadataUpdate,
    },
};

#[test]
fn deleted_trash_entries_refresh_the_trash_root() {
    let entry = FileEntry {
        location: Location::uri("trash:///photo.jpg"),
        native_name: "photo.jpg".into(),
        display_name: "photo.jpg".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(10),
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    };

    assert_eq!(
        deletion_parent_location(&entry.location),
        Some(Location::uri("trash:///"))
    );
}

#[test]
fn invalid_new_folder_names_are_rejected_before_an_operation_starts() {
    assert_invalid_creation_is_rejected(|browser| {
        browser.create_directory(Location::local("/fixture"), "../escaped".to_owned());
    });
}

#[test]
fn invalid_new_file_names_are_rejected_before_an_operation_starts() {
    assert_invalid_creation_is_rejected(|browser| {
        browser.create_file(Location::local("/fixture"), "../escaped".to_owned());
    });
}

fn assert_invalid_creation_is_rejected(create: impl FnOnce(&Rc<Browser>)) {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    create(&browser);

    assert_eq!(browser.current_operation.get(), None);
    assert!(browser.operation_load.borrow().is_none());
    assert!(matches!(
        events.borrow().as_slice(),
        [BrowserEvent::OperationFailed { message }] if message == "Names cannot contain /"
    ));
}

struct FakeFileSource;

struct RestoredSortingSource;

struct FilePreviewSource;

struct RejectingFileSource;

struct NotMountedFileSource;

struct RetryFileSource {
    attempts: Rc<Cell<usize>>,
}

struct TrackingFileSource {
    cancellations: Rc<Cell<usize>>,
}

struct RecordingFileSource {
    request_count: Rc<Cell<usize>>,
}

type WatchCallback = Rc<dyn Fn(DirectoryChange)>;

struct WatchingFileSource {
    notify: Rc<RefCell<Option<WatchCallback>>>,
}

impl FileSource for WatchingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![FileEntry {
                location: Location::local("/fixture/child"),
                native_name: OsString::from("child"),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.notify.replace(Some(notify));
        Some(LoadHandle::new(|| {}))
    }
}

impl FileSource for RecordingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        _request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        self.request_count.set(self.request_count.get() + 1);
        LoadHandle::new(|| {})
    }
}

impl FileSource for TrackingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        _request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let cancellations = self.cancellations.clone();
        LoadHandle::new(move || cancellations.set(cancellations.get() + 1))
    }
}

impl FileSource for RetryFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let attempt = self.attempts.get();
        self.attempts.set(attempt + 1);
        if attempt == 0 {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "temporarily unavailable".into(),
            });
        } else {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![FileEntry {
                    location: Location::local("/fixture/recovered"),
                    native_name: OsString::from("recovered"),
                    display_name: "recovered".into(),
                    kind: EntryKind::Directory,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                }],
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
            });
        }
        LoadHandle::new(|| {})
    }
}

impl FileSource for RejectingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Err(LocationValidationError::Inaccessible)
    }

    fn enumerate(
        &self,
        _request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

impl FileSource for NotMountedFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        Err(LocationValidationError::NotMounted(location.clone()))
    }

    fn enumerate(
        &self,
        _request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

impl FileSource for FilePreviewSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![FileEntry {
                location: Location::local("/fixture/example.conf"),
                native_name: OsString::from("example.conf"),
                display_name: "example.conf".into(),
                kind: EntryKind::File,
                size: MetadataValue::Known(12),
                modified_unix_seconds: MetadataValue::Known(1),
                is_hidden: false,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
}

impl FileSource for RestoredSortingSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let entry = |name: &str, size| FileEntry {
            location: Location::local(format!("/fixture/{name}")),
            native_name: OsString::from(name),
            display_name: name.to_owned(),
            kind: EntryKind::File,
            size: MetadataValue::Known(size),
            modified_unix_seconds: MetadataValue::Unknown,
            is_hidden: false,
        };
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![entry("small", 5), entry("large", 20)],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
}

impl FileSource for FakeFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![FileEntry {
                location: Location::local("/fixture/child"),
                native_name: OsString::from("child"),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
}

struct TrashFileSource;

impl FileSource for TrashFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![FileEntry {
                location: Location::uri("trash:///item"),
                native_name: OsString::from("item"),
                display_name: "item".into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
}

struct CountingFileSource {
    enumerate_calls: Rc<Cell<usize>>,
}

impl FileSource for CountingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.enumerate_calls.set(self.enumerate_calls.get() + 1);
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn large_restore_progress_defers_model_removal() {
    let browser = Browser::new(Rc::new(TrashFileSource));
    browser.navigate(Location::uri("trash:///"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    emit(OperationEvent::RestoreProgress {
        request_id,
        completed: 1,
        total: 3_000,
        restored_location: Some(Location::uri("trash:///item")),
    });

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::RestorationProgress {
            completed: 1,
            total: 3_000,
        }
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );
}

#[test]
fn cancellation_refreshes_an_affected_remote_root_and_its_open_descendants() {
    let enumerate_calls = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(CountingFileSource {
        enumerate_calls: enumerate_calls.clone(),
    }));
    let root = Location::uri("smb://host/share");
    browser.navigate(root.clone());
    browser.descend(0, Location::uri("smb://host/share/child"));
    assert_eq!(enumerate_calls.get(), 2);

    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let cancellations = Rc::new(Cell::new(0));
    let cancellations_for_handle = cancellations.clone();
    let request_id = browser.begin_operation();
    browser.deletion_operation.set(true);
    browser
        .operation_load
        .replace(Some(LoadHandle::new(move || {
            cancellations_for_handle.set(cancellations_for_handle.get() + 1);
        })));
    let emit = browser.operation_callback(request_id, false, HashSet::new());

    browser.cancel_file_operation();

    assert_eq!(cancellations.get(), 1);
    assert_eq!(browser.current_operation.get(), Some(request_id));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::DeletionFinished))
    );

    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: vec![Location::uri("smb://host/share/completed")],
            failed: vec![Location::uri("smb://host/share/interrupted")],
            not_attempted: vec![Location::uri("smb://host/share/not-attempted")],
            affected_locations: HashSet::from([root]),
        },
    });

    assert_eq!(browser.current_operation.get(), None);
    assert_eq!(enumerate_calls.get(), 2);
    let affected_locations = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            BrowserEvent::OperationCancelled {
                affected_locations, ..
            } => Some(affected_locations.clone()),
            _ => None,
        })
        .expect("cancellation event");
    browser.refresh_after_cancellation(&affected_locations);
    assert_eq!(enumerate_calls.get(), 4);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::DeletionFinished))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationCancelled {
            completed: 1,
            failed: 1,
            not_attempted: 1,
            ..
        }
    )));
}

#[test]
fn transfer_failure_reports_moves_completed_before_the_error() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(true));
    let emit = browser.operation_callback(request_id, false, HashSet::new());
    let completed = Location::local("/fixture/completed");

    emit(OperationEvent::TransferFailed {
        request_id,
        completed_locations: vec![completed.clone()],
        message: "injected failure".to_owned(),
    });

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::TransferFinished { moved_locations }
            if moved_locations == std::slice::from_ref(&completed)
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationFailed { message } if message == "injected failure"
    )));
}

struct ImmediateOperationProvider;

impl OperationProvider for ImmediateOperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Renamed {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        emit(OperationEvent::Created {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        emit(OperationEvent::Created {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: request.items.into_iter().map(|item| item.source).collect(),
        });
        LoadHandle::new(|| {})
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Deleted {
            request_id: request.id,
            locations: request
                .entries
                .into_iter()
                .map(|entry| entry.location)
                .collect(),
        });
        LoadHandle::new(|| {})
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Restored {
            request_id: request.id,
            locations: Vec::new(),
        });
        LoadHandle::new(|| {})
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Compressed {
            request_id: request.id,
            archive_name: request.archive_name,
        });
        LoadHandle::new(|| {})
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Extracted {
            request_id: request.id,
            first_name: None,
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn a_completed_trash_operation_can_be_undone_once() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let location = Location::local("/fixture/report.txt");
    let entry = FileEntry {
        location: location.clone(),
        native_name: OsString::from("report.txt"),
        display_name: "report.txt".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    };

    browser.delete(vec![entry], false);

    assert_eq!(
        pending_trash_undo().as_slice(),
        std::slice::from_ref(&location)
    );
    assert!(browser.undo_last_trash());
    assert!(!browser.undo_last_trash());
}

#[test]
fn another_browser_can_undo_the_latest_trash_operation() {
    let deleting_browser = Browser::new(Rc::new(FakeFileSource));
    deleting_browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let undoing_browser = Browser::new(Rc::new(FakeFileSource));
    undoing_browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let entry = FileEntry {
        location: Location::local("/fixture/report.txt"),
        native_name: OsString::from("report.txt"),
        display_name: "report.txt".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    };

    deleting_browser.delete(vec![entry], false);

    assert!(undoing_browser.undo_last_trash());
    assert!(!deleting_browser.undo_last_trash());
}

#[test]
fn permanent_delete_preserves_the_previous_trash_undo() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let trashed = FileEntry {
        location: Location::local("/fixture/report.txt"),
        native_name: OsString::from("report.txt"),
        display_name: "report.txt".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    };
    let permanently_deleted = FileEntry {
        location: Location::local("/fixture/draft.txt"),
        native_name: OsString::from("draft.txt"),
        display_name: "draft.txt".into(),
        ..trashed.clone()
    };

    browser.delete(vec![trashed], false);
    browser.delete(vec![permanently_deleted], true);

    assert!(browser.undo_last_trash());
}

#[test]
fn failed_and_partial_undo_operations_can_be_retried() {
    let first = Location::local("/fixture/first.txt");
    let second = Location::local("/fixture/second.txt");
    replace_pending_trash_undo(vec![first.clone(), second.clone()]);
    let (generation, _) = claim_pending_trash_undo().expect("undo claim");

    mark_trash_undo_restored(generation, &first);
    finish_trash_undo(generation, false);

    assert_eq!(pending_trash_undo(), vec![second.clone()]);
    let (retry_generation, retry) = claim_pending_trash_undo().expect("retry claim");
    assert_eq!(retry, vec![second]);
    finish_trash_undo(retry_generation, true);
}

#[test]
fn creating_a_directory_on_a_remote_location_refreshes_the_open_column() {
    let enumerate_calls = Rc::new(Cell::new(0));
    let source = CountingFileSource {
        enumerate_calls: enumerate_calls.clone(),
    };
    let browser = Browser::new(Rc::new(source));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.navigate(Location::uri("smb://host/share"));
    assert_eq!(enumerate_calls.get(), 1);

    browser.create_directory(Location::uri("smb://host/share"), "New Folder".to_owned());

    assert_eq!(
        enumerate_calls.get(),
        2,
        "a remote column has no live monitor, so it should be refreshed explicitly"
    );
}

#[test]
fn renaming_on_a_remote_location_refreshes_the_open_column() {
    let enumerate_calls = Rc::new(Cell::new(0));
    let source = CountingFileSource {
        enumerate_calls: enumerate_calls.clone(),
    };
    let browser = Browser::new(Rc::new(source));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.navigate(Location::uri("smb://host/share"));

    browser.rename(
        FileEntry {
            location: Location::uri("smb://host/share/old-name.txt"),
            native_name: "old-name.txt".into(),
            display_name: "old-name.txt".into(),
            kind: EntryKind::File,
            size: MetadataValue::Known(1),
            modified_unix_seconds: MetadataValue::Unknown,
            is_hidden: false,
        },
        "new-name.txt".to_owned(),
    );

    assert_eq!(enumerate_calls.get(), 2);
}

#[test]
fn creating_a_directory_locally_does_not_trigger_a_redundant_refresh() {
    let enumerate_calls = Rc::new(Cell::new(0));
    let source = CountingFileSource {
        enumerate_calls: enumerate_calls.clone(),
    };
    let browser = Browser::new(Rc::new(source));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser.navigate(Location::local("/fixture"));
    assert_eq!(enumerate_calls.get(), 1);

    browser.create_directory(Location::local("/fixture"), "New Folder".to_owned());

    assert_eq!(
        enumerate_calls.get(),
        1,
        "a local column already has a live file monitor; no extra refresh is needed"
    );
}

#[test]
fn restored_sorting_applies_to_the_initial_navigation_load() {
    let browser = Browser::with_preferences(
        Rc::new(RestoredSortingSource),
        ViewPreferences {
            sort_key: SortKey::Size,
            sort_direction: SortDirection::Descending,
            ..ViewPreferences::default()
        },
    );

    browser.navigate(Location::local("/fixture"));

    let snapshot = browser.column_snapshot(0).expect("initial column");
    assert_eq!(snapshot.selected_positions, Vec::<usize>::new());
    let names: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["large".to_owned(), "small".to_owned()]);
    assert_eq!(
        browser
            .column_preferences(0)
            .expect("initial column preferences")
            .sort_key,
        SortKey::Size
    );
}

#[test]
fn selecting_entries_by_name_preserves_the_full_matching_selection() {
    let browser = Browser::new(Rc::new(RestoredSortingSource));
    browser.navigate(Location::local("/fixture"));

    browser.select_entries_by_name(&["small".to_owned(), "large".to_owned()]);

    let snapshot = browser.column_snapshot(0).expect("initial column");
    let selected_names: Vec<_> = snapshot
        .selected_positions
        .iter()
        .map(|&position| {
            browser
                .entry_at(0, position)
                .expect("selected positions should resolve")
                .display_name
        })
        .collect();
    assert_eq!(selected_names, ["large", "small"]);
}
#[test]
fn navigation_events_are_delivered_to_every_observer() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let first_reset = Rc::new(Cell::new(false));
    let observed_first = first_reset.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_first.set(true);
        }
    });
    let second_reset = Rc::new(Cell::new(false));
    let observed_second = second_reset.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_second.set(true);
        }
    });

    browser.navigate(Location::local("/fixture"));

    assert!(first_reset.get());
    assert!(second_reset.get());
}

#[test]
fn filesystem_notifications_update_the_affected_column_incrementally() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Upsert(FileEntry {
        location: Location::local("/fixture/added"),
        native_name: OsString::from("added"),
        display_name: "added".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Known(1),
        is_hidden: false,
    }));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { depth: 0, splices, .. }
            if splices.len() == 1 && splices[0].removed == 0 && splices[0].entries.len() == 1
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            depth: 0,
            take_focus: false,
            ..
        }
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn ambiguous_filesystem_notifications_fall_back_to_reload() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Rescan);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

#[test]
fn column_snapshots_preserve_load_errors() {
    let browser = Browser::new(Rc::new(RetryFileSource {
        attempts: Rc::new(Cell::new(0)),
    }));

    browser.navigate(Location::local("/fixture"));

    let snapshot = browser.column_snapshot(0).expect("column should exist");
    assert_eq!(snapshot.error.as_deref(), Some("temporarily unavailable"));
    assert!(!snapshot.loading);
}

#[test]
fn retrying_a_failed_column_preserves_navigation_history() {
    let attempts = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RetryFileSource {
        attempts: attempts.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.retry_column(0);

    assert_eq!(attempts.get(), 2);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesReplaced { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::Reset))
    );
}

#[test]
fn hidden_file_preference_is_applied_to_reloaded_requests() {
    let request_count = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        request_count: request_count.clone(),
    }));
    let observed_preferences = Rc::new(Cell::new(None));
    let observed = observed_preferences.clone();
    browser.observe_preferences(move |preferences| observed.set(Some(preferences)));

    browser.navigate(Location::local("/fixture"));
    browser.toggle_hidden();

    // Toggling hidden files no longer re-enumerates; it only re-filters in-memory state.
    assert_eq!(request_count.get(), 1);
    assert_eq!(
        observed_preferences.get(),
        Some(ViewPreferences {
            show_hidden: true,
            ..ViewPreferences::default()
        })
    );
}

#[test]
fn navigating_away_cancels_the_previous_directory_request() {
    let cancellations = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(TrackingFileSource {
        cancellations: cancellations.clone(),
    }));

    browser.navigate(Location::local("/first"));
    browser.navigate(Location::local("/second"));

    assert_eq!(cancellations.get(), 1);
}

#[test]
fn navigating_to_the_active_location_is_a_noop() {
    let cancellations = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(TrackingFileSource {
        cancellations: cancellations.clone(),
    }));
    let resets = Rc::new(Cell::new(0));
    let observed_resets = resets.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_resets.set(observed_resets.get() + 1);
        }
    });

    browser.navigate(Location::uri("trash:///"));
    browser.navigate(Location::uri("trash:///"));

    assert_eq!(cancellations.get(), 0);
    assert_eq!(resets.get(), 1);
}

#[test]
fn deletion_targets_the_entered_folder_when_the_child_has_no_selection() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    browser.select(0, 0);
    browser.descend(0, Location::local("/fixture/child"));

    let entries = browser.deletion_entries();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].location, Location::local("/fixture/child"));
}

#[test]
fn completed_deletions_remove_entries_without_reloading_the_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.remove_deleted_locations(&[Location::local("/fixture/child")]);

    assert!(browser.entry_at(0, 0).is_none());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { splices, .. }
            if splices.iter().any(|splice| splice.removed == 1)
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn file_source_can_be_replaced_without_constructing_the_ui() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::local("/fixture"));

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesReplaced { count: 1, .. }))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
}

#[test]
fn valid_location_input_navigates_through_the_controller() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("/accepted"), Ok(()));

    assert_eq!(
        browser.active_location(),
        Some(Location::local("/accepted"))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnAdded { depth: 0, location }
            if location == &Location::local("/accepted")
    )));
}

#[test]
fn location_input_expands_trimmed_home_relative_paths() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let home = glib::home_dir();

    assert_eq!(browser.navigate_input("  ~  "), Ok(()));
    assert_eq!(browser.active_location(), Some(Location::local(&home)));

    assert_eq!(browser.navigate_input("  ~/Documents  "), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::local(home.join("Documents")))
    );
}

#[test]
fn home_relative_input_preserves_the_native_home_path() {
    let home = Path::new("/home/fixture");

    assert_eq!(
        location_from_input_with_home("~", home),
        Ok(Location::local("/home/fixture"))
    );
    assert_eq!(
        location_from_input_with_home("~/Documents/project", home),
        Ok(Location::local("/home/fixture/Documents/project"))
    );
    assert_eq!(
        location_from_input_with_home("~//Documents", home),
        Ok(Location::local("/home/fixture/Documents"))
    );
}

#[test]
fn other_users_home_shorthand_is_rejected() {
    assert!(matches!(
        location_from_input_with_home("~other-user/Documents", Path::new("/home/fixture")),
        Err(LocationValidationError::UnsupportedShorthand(_))
    ));
}

#[test]
fn sidebar_location_navigation_validates_uris_but_navigates_native_paths_directly() {
    let remote_browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    remote_browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    let remote = Location::uri("smb://host/share");
    remote_browser.navigate_location(remote.clone());

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::NotMounted(location)
        } if location == &remote
    )));
    assert_eq!(remote_browser.active_location(), None);

    let native_browser = Browser::new(Rc::new(RejectingFileSource));
    let native = Location::local("/saved/bookmark");
    native_browser.navigate_location(native.clone());

    assert_eq!(native_browser.active_location(), Some(native));
}

#[test]
fn location_input_accepts_uri_schemes_for_local_and_remote_locations() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    assert_eq!(browser.navigate_input("smb://192.168.1.220/share"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("smb://192.168.1.220/share"))
    );

    assert_eq!(browser.navigate_input("sftp://user@host:2222/path"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("sftp://user@host:2222/path"))
    );

    assert_eq!(browser.navigate_input("/regular/absolute/path"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::local("/regular/absolute/path"))
    );

    assert_eq!(browser.navigate_input("network:///"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("network:///"))
    );
}

#[test]
fn location_input_rejects_unsupported_uri_schemes() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    for uri in [
        "https://example.com/files",
        "file:///tmp",
        "custom://host/path",
    ] {
        assert!(matches!(
            browser.navigate_input(uri),
            Err(LocationValidationError::UnsupportedScheme(_))
        ));
        assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    }

    assert_eq!(browser.navigate_input("SMB://host/share"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("smb://host/share"))
    );
}

#[test]
fn location_input_rejects_unc_and_scp_shorthand_with_a_helpful_message() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    for shorthand in [
        r"\\host\share",
        r"smb:\\192.168.1.220",
        "//host/share",
        "//192.168.1.220",
        "user@host:path",
    ] {
        assert!(matches!(
            browser.navigate_input(shorthand),
            Err(LocationValidationError::UnsupportedShorthand(_))
        ));
        assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    }
}

#[test]
fn location_input_rejects_uris_with_an_embedded_password() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    for uri in [
        "smb://user:secret@host/share",
        "smb://user%3Asecret@host/share",
        "smb://user:sec%72et@host/share",
        "smb://user;password=secret@host/share",
        "smb://user%3Bpassword=secret@host/share",
        "smb://user%3Bpassword%3Dsecret@host/share",
        "smb://user;password=sec%72et@host/share",
        "sftp://user:secret@host:2222/path",
    ] {
        assert_eq!(
            browser.navigate_input(uri),
            Err(LocationValidationError::EmbeddedCredential)
        );
        assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    }

    assert_eq!(
        browser.navigate_input("smb://user%ZZ@host/share"),
        Err(LocationValidationError::InvalidUri)
    );
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));

    assert_eq!(
        browser.navigate_input("smb://user@host/share"),
        Ok(()),
        "a bare username without a password must still be accepted"
    );
}

#[test]
fn location_input_reports_the_target_location_when_not_mounted() {
    let browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("smb://192.168.1.220/share"), Ok(()));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::NotMounted(location)
        } if location == &Location::uri("smb://192.168.1.220/share")
    )));
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
}

#[test]
fn descending_into_an_unmounted_location_reports_it_for_retry() {
    let browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.descend(0, Location::uri("smb://192.168.1.220/share"));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::NavigationRejected {
            parent_depth: 0,
            error: LocationValidationError::NotMounted(location)
        } if location == &Location::uri("smb://192.168.1.220/share")
    )));
}

#[test]
fn rejected_directory_activation_preserves_navigation_state() {
    let browser = Browser::new(Rc::new(RejectingFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.descend(0, Location::local("/fixture/restricted"));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::NavigationRejected { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn rejected_location_input_preserves_navigation_state() {
    let browser = Browser::new(Rc::new(RejectingFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(
        browser.navigate_input("/restricted"),
        Err(LocationValidationError::Inaccessible)
    );

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(events.borrow().is_empty());
}

#[test]
fn invalid_location_text_is_rejected_before_the_provider() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    assert_eq!(
        browser.navigate_input(""),
        Err(LocationValidationError::Empty)
    );
    assert_eq!(
        browser.navigate_input("   "),
        Err(LocationValidationError::Empty)
    );
    assert_eq!(
        browser.navigate_input("relative/path"),
        Err(LocationValidationError::NotAbsolute)
    );
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
}

#[test]
fn peeking_streams_results_without_committing_navigation_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    browser.begin_peek(0, Location::local("/fixture/child"));

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekStarted { .. }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::PeekEntriesAdded { entries } if entries.len() == 1
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekFinished))
    );

    browser.back();
    let resets = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::Reset))
        .count();
    assert_eq!(resets, 1, "a peek must not create a history entry");
}

#[test]
fn an_already_open_child_is_not_peeked() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let child = Location::local("/fixture/child");
    browser.navigate(Location::local("/fixture"));
    browser.descend(0, child.clone());
    events.borrow_mut().clear();

    browser.begin_peek(0, child);

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekStarted { .. }))
    );
}

#[test]
fn committing_a_peek_descends_and_creates_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.begin_peek(0, Location::local("/fixture/child"));

    browser.commit_peek();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnAdded { depth: 1, location }
            if location == &Location::local("/fixture/child")
    )));
    browser.back();
    let resets = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::Reset))
        .count();
    assert_eq!(resets, 2, "committing a peek must create a history entry");
}

#[test]
fn single_click_action_descends_into_directories() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn activating_an_open_list_item_closes_its_child_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.preview(0, 0);
    assert_eq!(browser.active_depth(), Some(1));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { len: 1 }))
    );
    assert_eq!(browser.active_depth(), Some(0));
}

#[test]
fn explorer_activation_replaces_the_directory_instead_of_adding_a_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.activate_in_place(0, 0);

    assert_eq!(
        browser.active_location(),
        Some(Location::local("/fixture/child"))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn open_folder_remains_the_rename_target_until_its_pane_has_a_selection() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    browser.preview(0, 0);

    let (depth, position, entry) = browser.rename_item().expect("open folder rename target");
    assert_eq!((depth, position), (0, 0));
    assert_eq!(entry.location, Location::local("/fixture/child"));
}

#[test]
fn preview_and_open_are_distinct_file_actions() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::PreviewRequested { entry }
            if entry.location == Location::local("/fixture/example.conf")
    )));
    events.borrow_mut().clear();

    browser.activate(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/example.conf")
    )));
}

#[test]
fn keyboard_selection_and_activation_descend_without_the_ui() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    browser.move_selection(1);
    browser.activate_focused();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::FocusChanged {
            depth: 0,
            position: Some(0)
        }
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );

    browser.focus_parent();
    events.borrow_mut().clear();
    browser.activate_focused();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::FocusChanged {
            depth: 1,
            position: Some(0)
        }
    )));
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnsTruncated { .. } | BrowserEvent::ColumnAdded { .. }
    )));
}

#[test]
fn escape_closes_a_peek_before_the_deepest_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    browser.activate_focused();
    browser.begin_peek(1, Location::local("/fixture/child/child"));
    events.borrow_mut().clear();

    browser.escape();
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekClosed))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { .. }))
    );

    events.borrow_mut().clear();
    browser.escape();
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { len: 1 }))
    );
}

type CapturedLoad = Rc<RefCell<Option<(RequestId, Rc<dyn Fn(DirectoryEvent)>)>>>;

struct BatchReplaySource {
    captured: CapturedLoad,
}

impl FileSource for BatchReplaySource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.captured.replace(Some((request.id, emit)));
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        None
    }
}

fn batch_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn repeated_identical_batches_emit_selection_only_once() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    let batch = |names: &[&str]| DirectoryEvent::Batch {
        request_id,
        entries: names.iter().map(|name| batch_entry(name)).collect(),
    };
    emit(batch(&["alpha", "beta"]));
    browser.select(0, 0);
    emit(batch(&["mike"]));
    emit(batch(&["zulu"]));
    browser.flush_coalesced_capped(None);

    let selections: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::SelectionSetChanged {
                positions, focused, ..
            } => Some((positions.clone(), *focused)),
            _ => None,
        })
        .collect();
    assert_eq!(selections, vec![(vec![0], 0)]);
    let inserted = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::EntriesInserted { .. }))
        .count();
    assert_eq!(inserted, 2);
}

struct SortFillSource;

impl FileSource for SortFillSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![batch_entry("alpha"), batch_entry("beta")],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        emit(DirectoryEvent::MetadataFilled {
            request_id: request.id,
            updates: request
                .entries
                .iter()
                .map(|location| {
                    let size = if location.display_path().ends_with("beta") {
                        1
                    } else {
                        100
                    };
                    MetadataUpdate {
                        location: location.clone(),
                        size: MetadataValue::Known(size),
                        modified_unix_seconds: MetadataValue::Unknown,
                    }
                })
                .collect(),
        });
        emit(DirectoryEvent::MetadataFinished {
            request_id: request.id,
            outcome: MetadataOutcome::Complete,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        None
    }
}
#[test]
fn size_sort_waits_for_its_metadata_pass() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let browser = Browser::new(Rc::new(SortFillSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
    let observed = events.clone();
    let observed_waker = waker.clone();
    browser.observe(move |event| {
        let finished = matches!(event, BrowserEvent::SortingFinished { .. });
        observed.borrow_mut().push(event.clone());
        if finished && let Some(waker) = observed_waker.borrow_mut().take() {
            waker.wake();
        }
    });

    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    gtk::glib::MainContext::default().block_on(std::future::poll_fn(|cx| {
        let done = events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SortingFinished { .. }));
        if done || std::time::Instant::now() >= deadline {
            std::task::Poll::Ready(())
        } else {
            *waker.borrow_mut() = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    }));

    let names: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["beta".to_owned(), "alpha".to_owned()]);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::MetadataFilled { .. }) })
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| { matches!(event, BrowserEvent::SortingStarted { .. }) })
            .count(),
        1
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| { matches!(event, BrowserEvent::SortingFinished { .. }) })
            .count(),
        1
    );
}

#[test]
fn load_finish_applies_rows_queued_behind_the_count_threshold() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });

    let names: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) })
    );
}

#[test]
fn remote_load_finishes_only_after_every_queued_row_is_applied() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("first")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: (0..1025)
            .map(|index| batch_entry(&format!("queued-{index:04}")))
            .collect(),
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    assert_eq!(browser.state.borrow().columns[0].entries.len(), 513);

    browser.flush_coalesced_capped(Some(0));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    browser.flush_coalesced_capped(Some(0));

    let events = events.borrow();
    let finish = events
        .iter()
        .position(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
        .expect("the drained load should finish");
    let last_insert = events
        .iter()
        .rposition(|event| matches!(event, BrowserEvent::EntriesInserted { .. }))
        .expect("the final queued rows should be inserted");
    assert!(finish > last_insert);
    assert_eq!(browser.state.borrow().columns[0].entries.len(), 1026);
}

#[test]
fn remote_load_failure_waits_for_queued_rows() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("first")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: (0..513)
            .map(|index| batch_entry(&format!("queued-{index:04}")))
            .collect(),
    });
    emit(DirectoryEvent::Failed {
        request_id,
        message: "remote failure".to_owned(),
    });

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFailed { .. }))
    );
    browser.flush_coalesced_capped(Some(0));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LoadFailed { message, .. } if message == "remote failure"
    )));
    assert_eq!(browser.state.borrow().columns[0].entries.len(), 514);
}

enum FillAnswer {
    Complete(Vec<(&'static str, u64)>),
    Chunks(Vec<Vec<(&'static str, u64)>>, MetadataOutcome),
    EmptyComplete,
    TerminalOnly(MetadataOutcome),
    Never,
}

struct FillCall {
    id: RequestId,
    full: bool,
    entries: Vec<Location>,
    emit: DirectoryEmit,
}

type DirectoryEmit = Rc<dyn Fn(DirectoryEvent)>;

struct ScriptedSource {
    files: Vec<&'static str>,
    dirs: Vec<&'static str>,
    uri_base: Option<&'static str>,
    script: RefCell<Vec<FillAnswer>>,
    fill_calls: RefCell<Vec<FillCall>>,
    enumerate_calls: RefCell<Vec<(RequestId, DirectoryEmit)>>,
    manual_enumerate: bool,
}

impl ScriptedSource {
    fn scripted(files: Vec<&'static str>, script: Vec<FillAnswer>) -> Self {
        Self {
            files,
            dirs: Vec::new(),
            uri_base: None,
            script: RefCell::new(script),
            fill_calls: RefCell::new(Vec::new()),
            enumerate_calls: RefCell::new(Vec::new()),
            manual_enumerate: false,
        }
    }

    fn manual(files: Vec<&'static str>, script: Vec<FillAnswer>) -> Self {
        Self {
            files,
            dirs: Vec::new(),
            uri_base: None,
            script: RefCell::new(script),
            fill_calls: RefCell::new(Vec::new()),
            enumerate_calls: RefCell::new(Vec::new()),
            manual_enumerate: true,
        }
    }

    fn entry_location(&self, name: &str) -> Location {
        match self.uri_base {
            Some(base) => Location::uri(format!("{base}/{name}")),
            None => Location::local(format!("/fixture/{name}")),
        }
    }

    fn listed_entry(&self, name: &'static str, is_dir: bool) -> FileEntry {
        FileEntry {
            location: self.entry_location(name),
            native_name: std::ffi::OsString::from(name),
            display_name: name.into(),
            kind: if is_dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            is_hidden: false,
        }
    }
    fn answer(
        id: RequestId,
        uri_base: Option<&str>,
        answer: FillAnswer,
        emit: &Rc<dyn Fn(DirectoryEvent)>,
    ) {
        let locate = |name: &str| match uri_base {
            Some(base) => Location::uri(format!("{base}/{name}")),
            None => Location::local(format!("/fixture/{name}")),
        };
        let chunk = |rows: &[(&'static str, u64)]| DirectoryEvent::MetadataFilled {
            request_id: id,
            updates: rows
                .iter()
                .map(|(name, size)| MetadataUpdate {
                    location: locate(name),
                    size: MetadataValue::Known(*size),
                    modified_unix_seconds: MetadataValue::Known(7),
                })
                .collect(),
        };
        match answer {
            FillAnswer::Complete(rows) => {
                emit(chunk(&rows));
                emit(DirectoryEvent::MetadataFinished {
                    request_id: id,
                    outcome: MetadataOutcome::Complete,
                });
            }
            FillAnswer::Chunks(chunks, outcome) => {
                for rows in &chunks {
                    emit(chunk(rows));
                }
                emit(DirectoryEvent::MetadataFinished {
                    request_id: id,
                    outcome,
                });
            }
            FillAnswer::EmptyComplete => emit(DirectoryEvent::MetadataFinished {
                request_id: id,
                outcome: MetadataOutcome::Complete,
            }),
            FillAnswer::TerminalOnly(outcome) => emit(DirectoryEvent::MetadataFinished {
                request_id: id,
                outcome,
            }),
            FillAnswer::Never => {}
        }
    }
}

impl FileSource for ScriptedSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn supports_metadata_fill(&self, _location: &Location) -> bool {
        true
    }
    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if self.manual_enumerate {
            self.enumerate_calls.borrow_mut().push((request.id, emit));
            return LoadHandle::new(|| {});
        }
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: self
                .files
                .iter()
                .map(|name| self.listed_entry(name, false))
                .chain(self.dirs.iter().map(|name| self.listed_entry(name, true)))
                .collect(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let id = request.id;
        let full = request.full;
        let entries = request.entries.clone();
        let answer = self.script.borrow_mut().pop();
        match answer {
            Some(FillAnswer::Never) | None => {
                self.fill_calls.borrow_mut().push(FillCall {
                    id,
                    full,
                    entries,
                    emit,
                });
            }
            Some(answer) => {
                self.fill_calls.borrow_mut().push(FillCall {
                    id,
                    full,
                    entries,
                    emit: emit.clone(),
                });
                Self::answer(id, self.uri_base, answer, &emit);
            }
        }
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        None
    }
}

fn scripted_browser(
    source: ScriptedSource,
) -> (
    Rc<Browser>,
    Rc<RefCell<Vec<BrowserEvent>>>,
    Rc<ScriptedSource>,
) {
    let source = Rc::new(source);
    let browser = Browser::new(source.clone());
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    (browser, events, source)
}

/// The deadline only turns a hang into a deterministic failure.
fn pump_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() && std::time::Instant::now() < deadline {
        gtk::glib::MainContext::default().iteration(true);
    }
}

fn finish_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::SortingFinished { .. }))
        .count()
}

fn start_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::SortingStarted { .. }))
        .count()
}

fn replaced_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::EntriesReplaced { .. }))
        .count()
}

fn column_names(browser: &Browser, depth: usize) -> Vec<String> {
    browser.state.borrow().columns[depth]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect()
}

#[test]
fn multi_chunk_fill_sorts_once_on_its_terminal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Chunks(
            vec![vec![("alpha", 30)], vec![("beta", 10), ("gamma", 20)]],
            MetadataOutcome::Complete,
        )],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 2);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["beta".to_owned(), "gamma".to_owned(), "alpha".to_owned()]
    );
}

#[test]
fn truncated_fill_preserves_the_prior_order() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Chunks(
            vec![vec![("alpha", 30)]],
            MetadataOutcome::Truncated,
        )],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 1);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()]
    );
}

#[test]
fn unsupported_fill_abandons_the_sort_in_order() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::TerminalOnly(MetadataOutcome::Unsupported)],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 1);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
}

#[test]
fn failed_fill_abandons_the_sort_in_order() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Chunks(
            vec![vec![("alpha", 30), ("beta", 10)]],
            MetadataOutcome::Failed,
        )],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 1);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
}

#[test]
fn empty_fill_still_closes_the_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::EmptyComplete],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(start_count(&events), 1);
    assert_eq!(replaced_count(&events), 2);
}

#[test]
fn navigation_cancels_an_awaiting_sort_without_stale_commit() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let published = replaced_count(&events);
    assert_eq!(published, 1);
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    assert_eq!(start_count(&events), 1);

    browser.navigate(Location::local("/elsewhere"));
    assert_eq!(finish_count(&events), 1);
    assert!(browser.sort_awaiting_fill.borrow().is_none());
    assert!(browser.pending_sort.get().is_none());

    let old = source.fill_calls.borrow();
    let old_emit = old[0].emit.clone();
    let old_id = old[0].id;
    drop(old);
    old_emit(DirectoryEvent::MetadataFilled {
        request_id: old_id,
        updates: vec![MetadataUpdate {
            location: Location::local("/fixture/alpha"),
            size: MetadataValue::Known(1),
            modified_unix_seconds: MetadataValue::Known(7),
        }],
    });
    old_emit(DirectoryEvent::MetadataFinished {
        request_id: old_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), published + 1);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
}

#[test]
fn reload_cancels_an_awaiting_sort_and_ignores_its_terminal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let published = replaced_count(&events);
    assert_eq!(published, 1);
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());

    browser.reload_active();
    assert_eq!(finish_count(&events), 1);
    assert!(browser.sort_awaiting_fill.borrow().is_none());

    let old = source.fill_calls.borrow();
    let old_emit = old[0].emit.clone();
    let old_id = old[0].id;
    drop(old);
    old_emit(DirectoryEvent::MetadataFinished {
        request_id: old_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), published + 1);
    assert_eq!(finish_count(&events), 1);
}

#[test]
fn viewport_flush_never_disturbs_an_active_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never, FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.metadata_pending.borrow_mut().insert(
        0,
        vec![
            ViewportTarget {
                position: 0,
                location: Location::local("/fixture/alpha"),
            },
            ViewportTarget {
                position: 1,
                location: Location::local("/fixture/beta"),
            },
        ],
    );
    browser.flush_metadata_fills();
    assert_eq!(source.fill_calls.borrow().len(), 1);
    assert!(!source.fill_calls.borrow()[0].full);

    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| source.fill_calls.borrow().len() == 2);
    let calls = source.fill_calls.borrow();
    let viewport_id = calls[0].id;
    let viewport_emit = calls[0].emit.clone();
    let sort_id = calls[1].id;
    let sort_emit = calls[1].emit.clone();
    assert!(calls[1].full);
    assert_ne!(viewport_id, sort_id);
    drop(calls);

    viewport_emit(DirectoryEvent::MetadataFilled {
        request_id: viewport_id,
        updates: vec![MetadataUpdate {
            location: Location::local("/fixture/alpha"),
            size: MetadataValue::Known(30),
            modified_unix_seconds: MetadataValue::Known(7),
        }],
    });
    viewport_emit(DirectoryEvent::MetadataFinished {
        request_id: viewport_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), 1);
    assert_eq!(finish_count(&events), 0);
    assert!(browser.sort_awaiting_fill.borrow().is_some());

    sort_emit(DirectoryEvent::MetadataFilled {
        request_id: sort_id,
        updates: vec![
            MetadataUpdate {
                location: Location::local("/fixture/alpha"),
                size: MetadataValue::Known(30),
                modified_unix_seconds: MetadataValue::Known(7),
            },
            MetadataUpdate {
                location: Location::local("/fixture/beta"),
                size: MetadataValue::Known(10),
                modified_unix_seconds: MetadataValue::Known(7),
            },
        ],
    });
    sort_emit(DirectoryEvent::MetadataFinished {
        request_id: sort_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), 2);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["beta".to_owned(), "alpha".to_owned()]
    );
}

#[test]
fn refresh_drops_staging_and_its_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    assert_eq!(
        browser
            .staging
            .borrow()
            .get(&0)
            .map(|staged| staged.entries.len()),
        Some(2)
    );
    assert_eq!(replaced_count(&events), 0);

    browser.refresh_column(0);
    assert!(!browser.staging.borrow().contains_key(&0));
    assert_eq!(replaced_count(&events), 0);

    let (request_id, emit) = source.enumerate_calls.borrow()[1].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });
    assert_eq!(replaced_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );

    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    assert!(browser.sort_awaiting_fill.borrow().is_some());
    browser.refresh_column(0);
    assert!(browser.sort_awaiting_fill.borrow().is_none());
    assert!(browser.pending_sort.get().is_none());
    assert_eq!(start_count(&events), 1);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(replaced_count(&events), 1);
}

#[test]
fn close_column_clears_the_truncated_depth() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });

    browser.descend(0, Location::local("/fixture/sub"));
    let (sub_id, sub_emit) = source.enumerate_calls.borrow()[1].clone();
    sub_emit(DirectoryEvent::Batch {
        request_id: sub_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    sub_emit(DirectoryEvent::Finished {
        request_id: sub_id,
        truncated: false,
        can_trash: None,
    });
    let published = replaced_count(&events);
    assert_eq!(published, 2);

    browser.set_sort(1, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    assert!(browser.sort_awaiting_fill.borrow().is_some());

    browser.descend(1, Location::local("/fixture/sub/deep"));
    let (deep_id, deep_emit) = source.enumerate_calls.borrow()[2].clone();
    deep_emit(DirectoryEvent::Batch {
        request_id: deep_id,
        entries: vec![batch_entry("alpha")],
    });
    assert!(browser.staging.borrow().contains_key(&2));

    browser.close_column(1);
    assert!(!browser.staging.borrow().contains_key(&2));
    assert!(!browser.metadata_pending.borrow().contains_key(&1));
    assert!(!browser.sort_loads.borrow().contains_key(&1));
    assert!(browser.sort_awaiting_fill.borrow().is_none());
    assert!(browser.pending_sort.get().is_none());
    assert_eq!(start_count(&events), 1);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(replaced_count(&events), published);
}

#[test]
fn settle_timer_restarts_while_rows_keep_arriving() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"));
    let pump_until_elapsed = |millis: u64, start: std::time::Instant| {
        while start.elapsed() < std::time::Duration::from_millis(millis) {
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    let start = std::time::Instant::now();
    pump_until_elapsed(80, start);
    browser.request_metadata_fill(0, 1, Location::local("/fixture/beta"));
    pump_until_elapsed(140, start);
    assert!(source.fill_calls.borrow().is_empty());
    pump_until_elapsed(400, start);
    assert_eq!(source.fill_calls.borrow().len(), 1);
    assert!(!source.fill_calls.borrow()[0].full);
}

#[test]
fn shifted_viewport_rows_go_stale_without_repaint() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.metadata_pending.borrow_mut().insert(
        0,
        vec![ViewportTarget {
            position: 1,
            location: Location::local("/fixture/beta"),
        }],
    );
    browser.flush_metadata_fills();
    assert_eq!(source.fill_calls.borrow().len(), 1);

    browser.state.borrow_mut().columns[0].entries.remove(0);
    let fill = source.fill_calls.borrow();
    let emit = fill[0].emit.clone();
    let id = fill[0].id;
    drop(fill);
    emit(DirectoryEvent::MetadataFilled {
        request_id: id,
        updates: vec![MetadataUpdate {
            location: Location::local("/fixture/beta"),
            size: MetadataValue::Known(10),
            modified_unix_seconds: MetadataValue::Known(7),
        }],
    });
    assert_eq!(replaced_count(&events), 1);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::MetadataFilled { .. }) })
    );
    assert_eq!(
        browser.state.borrow().columns[0].entries[0].size,
        MetadataValue::Unknown
    );
}

#[test]
fn remote_name_listing_fills_visible_metadata() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let mut src = ScriptedSource::scripted(
        vec!["photo.jpg", "notes.txt"],
        vec![FillAnswer::Complete(vec![
            ("photo.jpg", 100),
            ("notes.txt", 50),
        ])],
    );
    src.uri_base = Some("sftp://host/share");
    let (browser, events, source) = scripted_browser(src);
    browser.navigate(Location::uri("sftp://host/share"));
    browser.metadata_pending.borrow_mut().insert(
        0,
        vec![
            ViewportTarget {
                position: 1,
                location: Location::uri("sftp://host/share/photo.jpg"),
            },
            ViewportTarget {
                position: 0,
                location: Location::uri("sftp://host/share/notes.txt"),
            },
        ],
    );
    browser.flush_metadata_fills();
    assert_eq!(source.fill_calls.borrow().len(), 1);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::MetadataFilled { .. }))
    );
    let sizes: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.size.clone())
        .collect();
    assert_eq!(
        sizes,
        vec![MetadataValue::Known(50), MetadataValue::Known(100)]
    );
}

#[test]
fn modified_sort_fills_directory_mtimes() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let mut src = ScriptedSource::scripted(vec!["b.txt"], vec![FillAnswer::Never]);
    src.dirs = vec!["sub"];
    let (browser, events, source) = scripted_browser(src);
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Modified, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    let fill = source.fill_calls.borrow();
    assert!(fill[0].full);
    assert!(fill[0].entries.contains(&Location::local("/fixture/sub")));
    let emit = fill[0].emit.clone();
    let id = fill[0].id;
    drop(fill);
    emit(DirectoryEvent::MetadataFilled {
        request_id: id,
        updates: vec![
            MetadataUpdate {
                location: Location::local("/fixture/sub"),
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Known(200),
            },
            MetadataUpdate {
                location: Location::local("/fixture/b.txt"),
                size: MetadataValue::Known(10),
                modified_unix_seconds: MetadataValue::Known(100),
            },
        ],
    });
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(finish_count(&events), 1);
    assert_eq!(replaced_count(&events), 2);
    let column = &browser.state.borrow().columns[0].entries;
    let sub = column
        .iter()
        .find(|entry| entry.location == Location::local("/fixture/sub"))
        .expect("the directory row should still be listed");
    assert_eq!(sub.modified_unix_seconds, MetadataValue::Known(200));
    assert_eq!(sub.size, MetadataValue::Unknown);
}

#[test]
fn fan_out_shares_one_event_with_every_observer() {
    let browser = Browser::new(Rc::new(SortFillSource));
    let first = Rc::new(RefCell::new(Vec::new()));
    let second = Rc::new(RefCell::new(Vec::new()));
    let third = Rc::new(RefCell::new(Vec::new()));
    for collected in [first.clone(), second.clone(), third.clone()] {
        browser.observe(move |event| collected.borrow_mut().push(event.clone()));
    }

    browser.navigate(Location::local("/fixture"));
    for collected in [first.clone(), second.clone(), third.clone()] {
        let published: Vec<_> = collected
            .borrow()
            .iter()
            .filter_map(|event| match event {
                BrowserEvent::EntriesReplaced { count, .. } => Some(*count),
                _ => None,
            })
            .collect();
        assert_eq!(published, vec![2]);
    }
    assert_eq!(first.borrow().len(), second.borrow().len());
    assert_eq!(second.borrow().len(), third.borrow().len());
}

#[test]
fn observers_added_or_cleared_mid_dispatch_do_not_corrupt_it() {
    let browser = Browser::new(Rc::new(SortFillSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    let late = Rc::new(RefCell::new(Vec::new()));
    let late_events = late.clone();
    let browser_for_observer = browser.clone();
    browser.observe(move |event| {
        observed.borrow_mut().push(event.clone());
        let value = late_events.clone();
        browser_for_observer.observe(move |event| {
            value.borrow_mut().push(event.clone());
        });
        browser_for_observer.clear_observer();
    });

    browser.navigate(Location::local("/fixture"));
    let first_wave = events.borrow().len();
    assert!(first_wave > 0);
    assert!(late.borrow().is_empty());

    browser.navigate(Location::local("/elsewhere"));
    assert_eq!(events.borrow().len(), first_wave);
}

#[test]
fn nested_emission_during_dispatch_is_safe() {
    let browser = Browser::new(Rc::new(SortFillSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    let browser_for_observer = browser.clone();
    browser.observe(move |event| {
        let select_now =
            matches!(event, BrowserEvent::EntriesReplaced { .. }) && observed.borrow().len() < 4;
        observed.borrow_mut().push(event.clone());
        if select_now {
            browser_for_observer.select(0, 0);
        }
    });

    browser.navigate(Location::local("/fixture"));
    assert!(
        events.borrow().iter().any(|event| {
            matches!(
                event,
                BrowserEvent::FocusChanged {
                    position: Some(0),
                    ..
                }
            )
        }),
        "the nested select should have been dispatched"
    );
}

fn staged_entry(name: &str, kind: EntryKind, size: MetadataValue<u64>, modified: i64) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: std::ffi::OsString::from(name),
        display_name: name.into(),
        kind,
        size,
        modified_unix_seconds: MetadataValue::Known(modified),
        is_hidden: false,
    }
}

#[test]
fn native_initial_load_publishes_sorted_once() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("gamma"), batch_entry("alpha")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    assert!(browser.staging.borrow().contains_key(&0));
    assert!(
        !events.borrow().iter().any(|event| {
            matches!(
                event,
                BrowserEvent::EntriesInserted { .. }
                    | BrowserEvent::EntriesPublished { .. }
                    | BrowserEvent::EntriesReplaced { .. }
            )
        }),
        "staging must not publish provisional rows"
    );
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });
    assert_eq!(replaced_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()]
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    assert!(browser.staging.borrow().is_empty());
}

#[test]
fn empty_native_initial_load_finishes_without_a_batch() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();

    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });

    assert_eq!(replaced_count(&events), 1);
    assert!(column_names(&browser, 0).is_empty());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LoadFinished {
            depth: 0,
            truncated: false
        }
    )));
}

#[test]
fn incomplete_native_metadata_uses_name_order_until_a_full_retry_finishes() {
    let source = Rc::new(ScriptedSource::manual(vec![], vec![FillAnswer::Never]));
    let browser = Browser::with_preferences(
        source.clone(),
        ViewPreferences {
            sort_key: SortKey::Size,
            ..ViewPreferences::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![
            staged_entry("beta", EntryKind::File, MetadataValue::Known(1), 1),
            staged_entry("alpha", EntryKind::File, MetadataValue::Unknown, 1),
        ],
    });
    emit(DirectoryEvent::MetadataIncomplete { request_id });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });

    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
    assert_eq!(start_count(&events), 1);
    assert!(browser.sort_awaiting_fill.borrow().is_some());
    let fills = source.fill_calls.borrow();
    assert_eq!(fills.len(), 1);
    assert!(fills[0].full);
    assert_ne!(fills[0].id, request_id);
}

#[test]
fn staged_load_reconciles_monitor_deltas_without_resurrection() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    browser.handle_directory_change(
        0,
        &Location::local("/fixture"),
        DirectoryChange::Remove(Location::local("/fixture/beta")),
    );
    browser.handle_directory_change(
        0,
        &Location::local("/fixture"),
        DirectoryChange::Upsert(batch_entry("gamma")),
    );
    assert!(
        !events.borrow().iter().any(|event| {
            matches!(
                event,
                BrowserEvent::EntriesSpliced { .. }
                    | BrowserEvent::EntriesPublished { .. }
                    | BrowserEvent::EntriesReplaced { .. }
            )
        }),
        "queued deltas must not touch the UI before the reconcile"
    );
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "gamma".to_owned()]
    );
}

#[test]
fn staged_sorts_order_every_key_in_both_directions() {
    use crate::model::{SortDirection, SortKey, ViewPreferences};
    let entries = || {
        vec![
            staged_entry("b.txt", EntryKind::File, MetadataValue::Known(20), 100),
            staged_entry("sub", EntryKind::Directory, MetadataValue::Unknown, 150),
            staged_entry("a.txt", EntryKind::File, MetadataValue::Known(10), 200),
        ]
    };
    let cases: &[(SortKey, SortDirection, bool, [&str; 3])] = &[
        (
            SortKey::Name,
            SortDirection::Ascending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Name,
            SortDirection::Ascending,
            false,
            ["a.txt", "b.txt", "sub"],
        ),
        (
            SortKey::Name,
            SortDirection::Descending,
            true,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Name,
            SortDirection::Descending,
            false,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Ascending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Ascending,
            false,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Descending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Descending,
            false,
            ["a.txt", "b.txt", "sub"],
        ),
        (
            SortKey::Size,
            SortDirection::Ascending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Size,
            SortDirection::Ascending,
            false,
            ["a.txt", "b.txt", "sub"],
        ),
        (
            SortKey::Size,
            SortDirection::Descending,
            true,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Size,
            SortDirection::Descending,
            false,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Ascending,
            true,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Ascending,
            false,
            ["b.txt", "sub", "a.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Descending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Descending,
            false,
            ["a.txt", "sub", "b.txt"],
        ),
    ];
    for (key, direction, folders_first, expected) in cases {
        let source: Rc<ScriptedSource> = Rc::new(ScriptedSource::manual(vec![], vec![]));
        let browser = Browser::with_preferences(
            source.clone(),
            ViewPreferences {
                sort_key: *key,
                sort_direction: *direction,
                folders_first: *folders_first,
                ..ViewPreferences::default()
            },
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        browser.navigate(Location::local("/fixture"));
        let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
        emit(DirectoryEvent::Batch {
            request_id,
            entries: entries(),
        });
        emit(DirectoryEvent::Finished {
            request_id,
            truncated: false,
            can_trash: None,
        });
        assert_eq!(
            column_names(&browser, 0),
            expected
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            "key {key:?} direction {direction:?} folders_first {folders_first}"
        );
        assert_eq!(replaced_count(&events), 1);
    }
}

#[test]
fn large_load_streams_prefix_then_tails_with_terminal_last() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    let first: Vec<FileEntry> = (0..400)
        .rev()
        .map(|index| batch_entry(&format!("file-{index:03}")))
        .collect();
    let second: Vec<FileEntry> = (400..700)
        .rev()
        .map(|index| batch_entry(&format!("file-{index:03}")))
        .collect();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: first,
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: second,
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });
    pump_until(|| {
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    });
    let kinds: Vec<(usize, usize)> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::EntriesReplaced { count, .. } => Some((0, *count)),
            BrowserEvent::EntriesPublished { count, .. } => Some((1, *count)),
            BrowserEvent::EntriesInserted { insertions, .. } => Some((
                1,
                insertions
                    .iter()
                    .map(|insertion| insertion.entries.len())
                    .sum(),
            )),
            BrowserEvent::LoadFinished { .. } => Some((2, 0)),
            _ => None,
        })
        .collect();
    assert!(!kinds.is_empty());
    assert_eq!(kinds[0], (0, 128));
    assert_eq!(
        *kinds.last().expect("a terminal should close the stream"),
        (2, 0)
    );
    let streamed: usize = kinds.iter().map(|(_, count)| count).sum();
    assert_eq!(streamed, 700);
    let names = column_names(&browser, 0);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert!(browser.staged_publishes.borrow().is_empty());
}

#[test]
fn remote_rows_flush_within_the_latency_bound() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::uri("sftp://host/share"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha")],
    });
    assert_eq!(column_names(&browser, 0), vec!["alpha".to_owned()]);
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    assert_eq!(column_names(&browser, 0), vec!["alpha".to_owned()]);
    let start = std::time::Instant::now();
    while column_names(&browser, 0).len() < 2 && start.elapsed() < std::time::Duration::from_secs(5)
    {
        gtk::glib::MainContext::default().iteration(true);
    }
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) }),
        "the load is still open; only the latency flush fired"
    );
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
    });
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) })
    );
}

#[test]
fn resort_after_mid_load_preference_change_republishes() {
    let (browser, events, _) =
        scripted_browser(ScriptedSource::scripted(vec!["b.txt", "a.txt"], vec![]));
    browser.navigate(Location::local("/fixture"));
    assert_eq!(
        column_names(&browser, 0),
        vec!["a.txt".to_owned(), "b.txt".to_owned()]
    );
    let request_id = browser
        .state
        .borrow()
        .request_id_for_depth(0)
        .expect("the load should still own its request");
    browser.sorting.borrow_mut().insert(
        0,
        SortingLoad {
            request_id,
            deltas: Vec::new(),
        },
    );
    let sorted = vec![batch_entry("b.txt"), batch_entry("a.txt")];
    let staged_preferences = ViewPreferences {
        sort_key: crate::model::SortKey::Size,
        ..ViewPreferences::default()
    };
    browser.finish_staged_sort(
        0,
        request_id,
        sorted,
        SortPlan {
            ordering_preferences: staged_preferences,
            staged_preferences,
            retry_metadata: false,
            truncated: false,
            can_trash: None,
        },
    );
    assert_eq!(
        column_names(&browser, 0),
        vec!["a.txt".to_owned(), "b.txt".to_owned()]
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) })
    );
}

struct MixedPeekFileSource;

impl FileSource for MixedPeekFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![
                FileEntry {
                    location: Location::local("/fixture/.dotfile"),
                    native_name: OsString::from(".dotfile"),
                    display_name: ".dotfile".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: true,
                },
                FileEntry {
                    location: Location::local("/fixture/normal.txt"),
                    native_name: OsString::from("normal.txt"),
                    display_name: "normal.txt".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                },
            ],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn peek_filters_hidden_entries_before_item_limit() {
    let browser = Browser::new(Rc::new(MixedPeekFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.begin_peek(0, Location::local("/fixture/peek_target"));

    let peek_batch = events.borrow().iter().find_map(|event| match event {
        BrowserEvent::PeekEntriesAdded { entries } => Some(entries.clone()),
        _ => None,
    });
    let entries = peek_batch.expect("peek batch emitted");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_name, "normal.txt");

    events.borrow_mut().clear();
    browser.toggle_hidden();
    browser.begin_peek(0, Location::local("/fixture/peek_target"));

    let peek_batch = events.borrow().iter().find_map(|event| match event {
        BrowserEvent::PeekEntriesAdded { entries } => Some(entries.clone()),
        _ => None,
    });
    let entries = peek_batch.expect("peek batch emitted");
    assert_eq!(entries.len(), 2);
}

#[test]
fn new_columns_inherit_show_hidden_preference() {
    let preferences = ViewPreferences {
        show_hidden: true,
        ..Default::default()
    };
    let browser = Browser::with_preferences(Rc::new(FakeFileSource), preferences);
    browser.navigate(Location::local("/fixture"));

    assert_eq!(
        browser.column_preferences(0).map(|p| p.show_hidden),
        Some(true)
    );

    browser.descend(0, Location::local("/fixture/child"));
    assert_eq!(
        browser.column_preferences(1).map(|p| p.show_hidden),
        Some(true)
    );
}
