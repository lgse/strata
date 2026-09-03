// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, ffi::OsString};

use super::*;
use crate::{
    model::{EntryKind, MetadataValue},
    services::{CancelledOperation, CompressRequest, ExtractRequest, LoadHandle},
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
    browser.observe(move |event| observed.borrow_mut().push(event));

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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["large", "small"]
    );
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
        .map(|&position| snapshot.entries[position].display_name.as_str())
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
fn retrying_a_failed_column_preserves_navigation_history() {
    let attempts = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RetryFileSource {
        attempts: attempts.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
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
            .any(|event| matches!(event, BrowserEvent::EntriesInserted { depth: 0, .. }))
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
    browser.observe(move |event| observed.borrow_mut().push(event));

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
    browser.observe(move |event| observed.borrow_mut().push(event));

    browser.navigate(Location::local("/fixture"));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesInserted { insertions, .. }
            if insertions.iter().map(|insertion| insertion.entries.len()).sum::<usize>() == 1
    )));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
fn sidebar_location_navigation_validates_uris_but_navigates_native_paths_directly() {
    let remote_browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    remote_browser.observe(move |event| observed.borrow_mut().push(event));

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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
fn committing_a_peek_descends_and_creates_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
    browser.observe(move |event| observed.borrow_mut().push(event));
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
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn peek_filters_hidden_entries_before_item_limit() {
    let browser = Browser::new(Rc::new(MixedPeekFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
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
