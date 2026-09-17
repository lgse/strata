// SPDX-License-Identifier: MIT

use gtk::prelude::FileExt;
use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
};

use super::*;

mod archive_activation;
mod camera_photos;
#[path = "deferred/tests.rs"]
mod deferred;
#[path = "directory_changes/tests.rs"]
mod directory_changes;
mod events;
mod location_input;
mod metadata;
mod monitor;
mod navigation;
#[path = "operation_events/tests.rs"]
mod operation_events;
mod operations;
mod preferences;
mod relocation;
mod selection;
#[path = "sorting/tests.rs"]
mod staged_sort;
mod staging;
mod undo;
mod undo_refresh;
use crate::{
    model::{EntryKind, MetadataValue},
    services::{
        CancelledOperation, CompressRequest, DirectoryEvent, ExtractRequest, LoadHandle,
        MetadataOutcome, MetadataRequest, MetadataUpdate, UndoCopyRequest, UndoMoveRequest,
    },
};

fn assert_invalid_creation_is_rejected(name: &str, create: impl FnOnce(&Rc<Browser>)) {
    let expected = validate_basename(name).expect_err("invalid fixture name");
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    create(&browser);

    assert_eq!(browser.current_operation.get(), None);
    assert!(browser.operation_load.borrow().is_none());
    assert!(matches!(
        events.borrow().as_slice(),
        [BrowserEvent::OperationFailed { message }] if message == expected
    ));
}

struct FakeFileSource;

struct RestoredSortingSource;

struct FilePreviewSource;

struct OpenChildBesideFileSource;

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
                thumbnail_path: None,
                location: Location::local("/fixture/child"),
                native_name: OsString::from("child"),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
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
                    thumbnail_path: None,
                    display_name: "recovered".into(),
                    kind: EntryKind::Directory,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                }],
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
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
                thumbnail_path: None,
                display_name: "example.conf".into(),
                kind: EntryKind::File,
                size: MetadataValue::Known(12),
                modified_unix_seconds: MetadataValue::Known(1),
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

struct ArchiveFileSource;

impl FileSource for ArchiveFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![
                FileEntry {
                    location: Location::local("/fixture/archive.zip"),
                    native_name: OsString::from("archive.zip"),
                    thumbnail_path: None,
                    display_name: "archive.zip".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Known(100),
                    modified_unix_seconds: MetadataValue::Known(1),
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
                FileEntry {
                    location: Location::local("/fixture/notes.txt"),
                    native_name: OsString::from("notes.txt"),
                    thumbnail_path: None,
                    display_name: "notes.txt".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Known(20),
                    modified_unix_seconds: MetadataValue::Known(1),
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
                FileEntry {
                    location: Location::uri("sftp://example.com/remote-archive.zip"),
                    native_name: OsString::from("remote-archive.zip"),
                    thumbnail_path: None,
                    display_name: "remote-archive.zip".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Known(50),
                    modified_unix_seconds: MetadataValue::Known(1),
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
            ],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

impl FileSource for OpenChildBesideFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let entries = if request.location == Location::local("/fixture") {
            vec![
                FileEntry {
                    location: Location::local("/fixture/child"),
                    native_name: OsString::from("child"),
                    thumbnail_path: None,
                    display_name: "child".into(),
                    kind: EntryKind::Directory,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
                FileEntry {
                    location: Location::local("/fixture/example.conf"),
                    native_name: OsString::from("example.conf"),
                    thumbnail_path: None,
                    display_name: "example.conf".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Known(12),
                    modified_unix_seconds: MetadataValue::Known(1),
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
            ]
        } else {
            Vec::new()
        };
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries,
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
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
            thumbnail_path: None,
            display_name: name.to_owned(),
            kind: EntryKind::File,
            size: MetadataValue::Known(size),
            modified_unix_seconds: MetadataValue::Unknown,
            is_hidden: false,
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        };
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![entry("small", 5), entry("large", 20)],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
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
                thumbnail_path: None,
                location: Location::local("/fixture/child"),
                native_name: OsString::from("child"),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
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
                thumbnail_path: None,
                display_name: "item".into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            }],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
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
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

thread_local! {
    static UNDO_MOVE_REQUESTS: RefCell<Vec<Vec<MoveRecord>>> = const { RefCell::new(Vec::new()) };
    static UNDO_COPY_REQUESTS: RefCell<Vec<Vec<Location>>> = const { RefCell::new(Vec::new()) };
    static UNDO_RENAME_REQUESTS: RefCell<Vec<(Location, Location)>> = const { RefCell::new(Vec::new()) };
    static FORWARD_RENAME_OUTCOME: Cell<Option<ForwardRenameOutcome>> = const { Cell::new(None) };
}

#[derive(Clone, Copy)]
enum ForwardRenameOutcome {
    Failed,
    Cancelled,
}

struct ImmediateOperationProvider;

impl OperationProvider for ImmediateOperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        match FORWARD_RENAME_OUTCOME.with(Cell::take) {
            Some(ForwardRenameOutcome::Failed) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: "rename failed".to_owned(),
            }),
            Some(ForwardRenameOutcome::Cancelled) => emit(OperationEvent::Cancelled {
                request_id: request.id,
                result: Default::default(),
            }),
            None => emit(OperationEvent::Renamed {
                request_id: request.id,
            }),
        }
        LoadHandle::new(|| {})
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        if request.unique_name {
            let child =
                crate::adapters::gio_file_for_location(&request.parent).child(&request.name);
            emit(OperationEvent::EntryCreated {
                request_id: request.id,
                location: crate::adapters::location_for_file(&child).expect("created location"),
            });
        } else {
            emit(OperationEvent::Created {
                request_id: request.id,
            });
        }
        LoadHandle::new(|| {})
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        if request.unique_name {
            let child =
                crate::adapters::gio_file_for_location(&request.parent).child(&request.name);
            emit(OperationEvent::EntryCreated {
                request_id: request.id,
                location: crate::adapters::location_for_file(&child).expect("created location"),
            });
        } else {
            emit(OperationEvent::Created {
                request_id: request.id,
            });
        }
        LoadHandle::new(|| {})
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        for (index, item) in request.items.iter().enumerate() {
            emit(OperationEvent::TransferProgress {
                request_id: request.id,
                completed_items: index + 1,
                transferred_bytes: 0,
                total_bytes: None,
                created_location: (!request.move_sources)
                    .then(|| item.source.transfer_target(&request.destination))
                    .flatten(),
            });
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: request.items.into_iter().map(|item| item.source).collect(),
        });
        LoadHandle::new(|| {})
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        UNDO_MOVE_REQUESTS.with(|requests| {
            requests.borrow_mut().push(
                request
                    .items
                    .iter()
                    .map(|item| item.record.clone())
                    .collect(),
            )
        });
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: request
                .items
                .into_iter()
                .map(|item| item.record.current)
                .collect(),
        });
        LoadHandle::new(|| {})
    }

    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        UNDO_RENAME_REQUESTS.with(|requests| {
            requests
                .borrow_mut()
                .push((request.current.clone(), request.original.clone()))
        });
        emit(OperationEvent::Renamed {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }

    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().push(request.locations.clone()));
        emit(OperationEvent::Deleted {
            request_id: request.id,
            locations: request.locations,
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

type OperationEmit = Rc<dyn Fn(OperationEvent)>;

struct HeldExtractProvider {
    cancelled: Rc<Cell<bool>>,
    emit: Rc<RefCell<Option<OperationEmit>>>,
    request_id: Rc<Cell<Option<OperationRequestId>>>,
}

impl OperationProvider for HeldExtractProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.rename(request, emit)
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.create_directory(request, emit)
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.create_file(request, emit)
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.paste(request, emit)
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.undo_move(request, emit)
    }

    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.undo_rename(request, emit)
    }

    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.undo_copy(request, emit)
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.delete(request, emit)
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.restore(request, emit)
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.compress(request, emit)
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.request_id.set(Some(request.id));
        self.emit.replace(Some(emit));
        let cancelled = self.cancelled.clone();
        LoadHandle::new(move || cancelled.set(true))
    }
}

fn fixture_entry(path: &str) -> FileEntry {
    let location = Location::local(path);
    let name = location.display_name();
    FileEntry {
        location,
        thumbnail_path: None,
        native_name: OsString::from(name.clone()),
        display_name: name,
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    }
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
        thumbnail_path: None,
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    }
}

fn trash_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::uri(format!("trash:///{name}")),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        ..batch_entry(name)
    }
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
            can_delete: None,
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
                        mode: MetadataValue::Unknown,
                        image_dimensions: MetadataValue::Unknown,
                        child_count: MetadataValue::Unknown,
                        duration_seconds: MetadataValue::Unknown,
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
            manual_enumerate: true,
            ..Self::scripted(files, script)
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
            thumbnail_path: None,
            display_name: name.into(),
            kind: if is_dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            is_hidden: name.starts_with('.'),
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
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
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
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
            can_delete: None,
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

fn staged_entry(name: &str, kind: EntryKind, size: MetadataValue<u64>, modified: i64) -> FileEntry {
    FileEntry {
        kind,
        size,
        modified_unix_seconds: MetadataValue::Known(modified),
        ..batch_entry(name)
    }
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
                    thumbnail_path: None,
                    display_name: ".dotfile".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: true,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
                FileEntry {
                    location: Location::local("/fixture/normal.txt"),
                    native_name: OsString::from("normal.txt"),
                    thumbnail_path: None,
                    display_name: "normal.txt".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
            ],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}
