// SPDX-License-Identifier: MIT

mod cancellation;
mod copies;
mod placement;
mod trash_entries;

use super::super::move_restore_path;
use super::fs_quirks::{Op, OverEmptyDirectory, Racer, Support};
use super::*;
use crate::services::{CancelledOperation, KeptCause, KeptItem, SpecialKind, TransferReport};
use rustix::io::Errno;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};

fn restore_path(
    source: &Path,
    destination: &Path,
    allowed: &Path,
) -> Result<TransferReport, glib::Error> {
    glib::MainContext::default()
        .block_on(move_restore_path(
            source.to_path_buf(),
            destination.to_path_buf(),
            allowed.to_path_buf(),
            Arc::default(),
            gio::Cancellable::new(),
        ))
        .map(|(report, _)| report)
}

/// A fixture with `source` and `destination` paths under a temporary root.
struct RestoreFixture {
    root: tempfile::TempDir,
    source: PathBuf,
    destination: PathBuf,
}

impl RestoreFixture {
    fn new() -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        Ok(Self {
            root,
            source,
            destination,
        })
    }

    /// Writes `original` to the source file, or to `contents` in the source folder.
    fn seed(&self, directory: bool) -> io::Result<()> {
        if directory {
            fs::create_dir(&self.source)?;
            fs::write(self.source.join("contents"), b"original")
        } else {
            fs::write(&self.source, b"original")
        }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn install(&self, quirks: Quirks) -> FsQuirks {
        FsQuirks::install(self.root.path(), quirks)
    }

    fn restore(&self) -> Result<TransferReport, glib::Error> {
        restore_path(&self.source, &self.destination, self.root.path())
    }

    fn restore_cancellable(&self, cancellable: gio::Cancellable) -> Result<(), glib::Error> {
        glib::MainContext::default()
            .block_on(move_restore_path(
                self.source.clone(),
                self.destination.clone(),
                self.root.path().to_path_buf(),
                Arc::default(),
                cancellable,
            ))
            .map(drop)
    }

    /// The path `seed` wrote `original` to.
    fn seeded(&self, directory: bool) -> PathBuf {
        if directory {
            self.source.join("contents")
        } else {
            self.source.clone()
        }
    }

    /// Where the seeded contents land once the source is restored.
    fn restored(&self, directory: bool) -> PathBuf {
        if directory {
            self.destination.join("contents")
        } else {
            self.destination.clone()
        }
    }
}

fn writes(path: &Path, contents: &'static [u8]) -> Racer {
    let path = path.to_path_buf();
    Box::new(move || {
        fs::write(&path, contents).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    })
}

/// Moves the source aside to `retained` and puts a replacement at its name.
fn replaces_source(source: &Path, retained: &Path) -> Racer {
    let (raced, retained) = (source.to_path_buf(), retained.to_path_buf());
    Box::new(move || {
        fs::rename(&raced, &retained).expect("retain original source");
        fs::write(&raced, b"replacement").expect("replace source");
    })
}

fn set_user_xattr(file: &fs::File) -> io::Result<()> {
    rustix::fs::fsetxattr(
        file,
        "user.strata-test",
        b"metadata",
        rustix::fs::XattrFlags::empty(),
    )
    .map_err(io::Error::from)
}

fn user_xattr(path: &Path) -> io::Result<Vec<u8>> {
    let mut value = [0u8; 32];
    let count = rustix::fs::getxattr(path, "user.strata-test", &mut value[..])?;
    Ok(value[..count].to_vec())
}

/// A volume's `.Trash-<uid>` with its `files` and `info` folders.
struct VolumeTrash {
    trash: PathBuf,
}

impl VolumeTrash {
    fn create(root: &Path) -> io::Result<Self> {
        let uid = rustix::process::getuid().as_raw();
        Self::create_at(root.join(format!(".Trash-{uid}")))
    }

    fn create_at(trash: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(trash.join("files"))?;
        fs::create_dir_all(trash.join("info"))?;
        Ok(Self { trash })
    }

    fn files(&self) -> PathBuf {
        self.trash.join("files")
    }

    fn file(&self, name: &str) -> PathBuf {
        self.files().join(name)
    }

    fn info(&self, name: &str) -> PathBuf {
        self.trash.join("info").join(format!("{name}.trashinfo"))
    }

    fn add_info(&self, name: &str, orig_path: &str) -> io::Result<()> {
        fs::write(self.info(name), format!("[Trash Info]\nPath={orig_path}\n"))
    }

    fn add_dated_info(&self, name: &str, orig_path: &str) -> io::Result<()> {
        fs::write(
            self.info(name),
            format!("[Trash Info]\nPath={orig_path}\nDeletionDate=2026-01-01T00:00:00\n"),
        )
    }
}

fn trashed_file(source: &Path, destination: &Path) -> RestoreTrashItem {
    RestoreTrashItem {
        entry: file_entry(source),
        destination: destination.to_path_buf(),
    }
}

fn trashed_directory(source: &Path, destination: &Path) -> RestoreTrashItem {
    RestoreTrashItem {
        entry: directory_entry(source),
        destination: destination.to_path_buf(),
    }
}

type Recorded = Rc<RefCell<Vec<OperationEvent>>>;

fn recording() -> (Recorded, Rc<dyn Fn(OperationEvent)>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    (
        events,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    )
}

fn start_restore(id: u64, items: Vec<RestoreTrashItem>) -> (LoadHandle, Recorded) {
    let (events, emit) = recording();
    let operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(id),
            source: RestoreSource::TrashEntries(items),
        },
        emit,
    );
    (operation, events)
}

/// Restores the Trash entries and returns every event up to the restore's end.
fn restore_entries(id: u64, items: Vec<RestoreTrashItem>) -> Vec<OperationEvent> {
    let (_operation, events) = start_restore(id, items);
    wait_for_restore(&events);
    events.take()
}

fn kept_in_trash(events: &[OperationEvent]) -> Vec<KeptItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.kept_in_trash.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn assert_restored(events: &[OperationEvent], row: &str) {
    assert!(
        matches!(events.last(), Some(OperationEvent::Restored { .. })),
        "{row}: {events:?}"
    );
}

fn assert_restore_failed(events: &[OperationEvent], needle: &str) {
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::RestoreCompletedWithErrors { message, .. })
                if message.contains(needle)
        ),
        "{events:?}"
    );
}

fn cancelled_result<'a>(events: &'a [OperationEvent], row: &str) -> &'a CancelledOperation {
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("{row}: expected a cancel, got {events:?}");
    };
    result
}

/// The items a cancel completed, failed and did not attempt.
fn outcome(result: &CancelledOperation) -> (Vec<Location>, Vec<Location>, Vec<Location>) {
    (
        result.completed.clone(),
        result.failed.clone(),
        result.not_attempted.clone(),
    )
}

fn expect_contents(expected: &[(PathBuf, &[u8])]) {
    for (path, contents) in expected {
        let read = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(read, *contents, "{}", path.display());
    }
}

/// Asserts that nothing is at the paths, following symlinks as `exists` does.
fn expect_gone(paths: &[&Path]) {
    for path in paths {
        assert!(!path.exists(), "{}", path.display());
    }
}

fn expect_no_entry(paths: &[&Path]) {
    for path in paths {
        assert!(fs::symlink_metadata(path).is_err(), "{}", path.display());
    }
}

fn expect_entries(paths: &[&Path]) {
    for path in paths {
        assert!(fs::symlink_metadata(path).is_ok(), "{}", path.display());
    }
}
