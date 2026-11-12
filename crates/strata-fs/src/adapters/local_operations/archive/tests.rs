// SPDX-License-Identifier: MIT

mod compressing;
mod failures;
mod layout;
mod round_trip;

use super::{
    ArchiveError, copy_with_big_buf,
    decoders::{extract_7z_from_reader, extract_tar},
    fixtures::{
        COMPRESSION_STAGE, EXTRACTION_STAGE, FixtureMember, HomeTrashGuard, corrupt_gzip_trailer,
        expected_mode, extract_zip, never_cancelled, set_times_without_following, stages,
        tempdir_on_home_device, test_file_entry, write_7z_entries, write_members,
        write_tar_entries, write_zip_stored,
    },
    listing::INVALID_ARCHIVE,
};
use crate::{
    adapters::local_operations::{
        LocalOperationProvider,
        tests::{
            base_places_files_by_linking,
            fs_quirks::{FsQuirks, Op, Preset, Quirks, Trash},
        },
    },
    model::Location,
    services::{
        ArchiveFormat, CompressRequest, ExtractRequest, KeptCause, KeptItem, LoadHandle,
        OperationEvent, OperationProvider, OperationRequestId, PasswordFailure, RenameCause,
        RenamedItem, ReplacedOriginal, SkippedItem, SpecialKind, TransferConflict, TransferReport,
    },
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};
use rustix::io::Errno;
use std::{
    cell::RefCell,
    error::Error,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};

type Emit = Rc<dyn Fn(OperationEvent)>;

/// Iterates the main context until an event `ends` the operation `start` began.
fn run_until<H>(
    start: impl FnOnce(Emit) -> H,
    ends: fn(&OperationEvent) -> bool,
) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = start(Rc::new(move |event| emitted.borrow_mut().push(event)));
    while !events.borrow().iter().any(ends) {
        glib::MainContext::default().iteration(true);
    }
    drop(operation);
    events.borrow().clone()
}

fn run_compression(request: CompressRequest) -> Vec<OperationEvent> {
    run_until(
        |emit| LocalOperationProvider.compress(request, emit),
        |event| {
            matches!(
                event,
                OperationEvent::Compressed { .. } | OperationEvent::Failed { .. }
            )
        },
    )
}

fn run_extraction(request: ExtractRequest) -> Vec<OperationEvent> {
    run_until(
        |emit| LocalOperationProvider.extract(request, emit),
        |event| {
            matches!(
                event,
                OperationEvent::Extracted { .. } | OperationEvent::Failed { .. }
            )
        },
    )
}

fn serial() -> Result<crate::test_support::MainContextGuard<'static>, String> {
    ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())
}

/// A ZIP of `source` named `archive_name` in `destination`.
fn zip_request(
    source: &Path,
    destination: &Path,
    archive_name: &str,
    conflict: TransferConflict,
) -> CompressRequest {
    CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(source)],
        destination: Location::local(destination),
        archive_name: archive_name.to_owned(),
        conflict,
        format: ArchiveFormat::Zip,
        password: None,
    }
}

/// An extraction of `archive` into a `destination` the caller did not make, without password.
fn extract_request(id: u64, archive: &Path, destination: &Path) -> ExtractRequest {
    ExtractRequest {
        id: OperationRequestId(id),
        entry: test_file_entry(archive),
        destination: Location::local(destination),
        created_destination: false,
        password: None,
    }
}

fn entry_names(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    names.sort();
    Ok(names)
}

fn expect_contents(expected: &[(PathBuf, &[u8])], context: &str) -> Result<(), Box<dyn Error>> {
    for (path, contents) in expected {
        assert_eq!(fs::read(path)?, *contents, "{context}: {}", path.display());
    }
    Ok(())
}

fn expect_failed(events: &[OperationEvent]) {
    assert!(
        matches!(events.last(), Some(OperationEvent::Failed { .. })),
        "{events:?}"
    );
}

fn expect_extracted_as(events: &[OperationEvent], first: &str) {
    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::Extracted { first_name: Some(name), .. }) if name == first
        ),
        "{first}: {events:?}"
    );
}
