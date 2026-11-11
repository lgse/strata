// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct FakeFileSource;

pub(super) struct RestoredSortingSource;

pub(super) struct FilePreviewSource;

pub(super) struct OpenChildBesideFileSource;

pub(super) struct RejectingFileSource;

pub(super) struct NotMountedFileSource;

pub(super) struct ArchiveFileSource;

pub(super) struct TrashFileSource;

pub(super) struct SortFillSource;

pub(super) struct MixedPeekFileSource;

pub(super) struct RetryFileSource {
    pub(super) attempts: Rc<Cell<usize>>,
}

pub(super) struct TrackingFileSource {
    pub(super) cancellations: Rc<Cell<usize>>,
}

pub(super) struct RecordingFileSource {
    pub(super) request_count: Rc<Cell<usize>>,
}

pub(super) struct WatchingFileSource {
    pub(super) notify: Rc<RefCell<Option<WatchCallback>>>,
}

pub(super) struct CountingFileSource {
    pub(super) enumerate_calls: Rc<Cell<usize>>,
}

pub(super) type CapturedLoad = Rc<RefCell<Option<(RequestId, Rc<dyn Fn(DirectoryEvent)>)>>>;

pub(super) struct BatchReplaySource {
    pub(super) captured: CapturedLoad,
}

/// Lists `entries` in one batch and finishes the load at once.
fn list_entries(
    request: &DirectoryRequest,
    emit: &Rc<dyn Fn(DirectoryEvent)>,
    entries: Vec<FileEntry>,
) {
    emit(DirectoryEvent::Batch {
        request_id: request.id,
        entries,
    });
    finish_listing(request, emit);
}

fn finish_listing(request: &DirectoryRequest, emit: &Rc<dyn Fn(DirectoryEvent)>) {
    emit(DirectoryEvent::Finished {
        request_id: request.id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
}

fn sized_file(path: &str, size: u64) -> FileEntry {
    FileEntry {
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Known(1),
        created_unix_seconds: MetadataValue::Unknown,
        ..fixture_entry(path)
    }
}

fn no_handle() -> LoadHandle {
    LoadHandle::new(|| {})
}

impl FileSource for WatchingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        list_entries(&request, &emit, vec![directory_entry("/fixture/child")]);
        no_handle()
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.notify.replace(Some(notify));
        Some(no_handle())
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
        no_handle()
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
                refused: false,
                denied: false,
            });
        } else {
            list_entries(&request, &emit, vec![directory_entry("/fixture/recovered")]);
        }
        no_handle()
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
        no_handle()
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
        no_handle()
    }
}

impl FileSource for FilePreviewSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        list_entries(
            &request,
            &emit,
            vec![sized_file("/fixture/example.conf", 12)],
        );
        no_handle()
    }
}

impl FileSource for ArchiveFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let remote_archive = FileEntry {
            size: MetadataValue::Known(50),
            modified_unix_seconds: MetadataValue::Known(1),
            created_unix_seconds: MetadataValue::Unknown,
            ..entry_at(
                Location::uri("sftp://example.com/remote-archive.zip"),
                EntryKind::File,
            )
        };
        let socket = FileEntry {
            kind: EntryKind::Other,
            ..sized_file("/fixture/socket.zip", 0)
        };
        let entries = vec![
            sized_file("/fixture/archive.zip", 100),
            sized_file("/fixture/notes.txt", 20),
            remote_archive,
            socket,
        ];
        list_entries(&request, &emit, entries);
        no_handle()
    }
}

impl FileSource for OpenChildBesideFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let entries = if request.location == Location::local("/fixture") {
            vec![
                directory_entry("/fixture/child"),
                sized_file("/fixture/example.conf", 12),
            ]
        } else {
            Vec::new()
        };
        list_entries(&request, &emit, entries);
        no_handle()
    }
}

impl FileSource for RestoredSortingSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let entry = |name: &str, size| FileEntry {
            size: MetadataValue::Known(size),
            ..batch_entry(name)
        };
        list_entries(&request, &emit, vec![entry("small", 5), entry("large", 20)]);
        no_handle()
    }
}

impl FileSource for FakeFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        list_entries(&request, &emit, vec![directory_entry("/fixture/child")]);
        no_handle()
    }
}

impl FileSource for TrashFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        list_entries(&request, &emit, vec![trash_entry("item")]);
        no_handle()
    }
}

impl FileSource for CountingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.enumerate_calls.set(self.enumerate_calls.get() + 1);
        finish_listing(&request, &emit);
        no_handle()
    }
}

impl FileSource for BatchReplaySource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.captured.replace(Some((request.id, emit)));
        no_handle()
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

impl FileSource for SortFillSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        list_entries(
            &request,
            &emit,
            vec![batch_entry("alpha"), batch_entry("beta")],
        );
        no_handle()
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        emit(DirectoryEvent::MetadataFilled {
            request_id: request.id,
            updates: request.entries.iter().map(sized_update).collect(),
        });
        emit(DirectoryEvent::MetadataFinished {
            request_id: request.id,
            outcome: MetadataOutcome::Complete,
        });
        no_handle()
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

/// `beta` is the small entry so a size sort reverses the listed order.
fn sized_update(location: &Location) -> MetadataUpdate {
    let size = if location.display_path().ends_with("beta") {
        1
    } else {
        100
    };
    MetadataUpdate {
        location: location.clone(),
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
    }
}

impl FileSource for MixedPeekFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let dotfile = FileEntry {
            is_hidden: true,
            ..batch_entry(".dotfile")
        };
        list_entries(&request, &emit, vec![dotfile, batch_entry("normal.txt")]);
        no_handle()
    }
}
