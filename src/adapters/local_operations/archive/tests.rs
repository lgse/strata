// SPDX-License-Identifier: MIT

use super::{
    ArchiveError, copy_with_big_buf,
    decoders::{extract_7z_from_reader, extract_tar},
    fixtures::{
        compression_stages, extract_zip, never_cancelled, test_file_entry, write_zip_stored,
    },
};
use crate::{
    adapters::local_operations::LocalOperationProvider,
    model::Location,
    services::{
        ArchiveFormat, CompressRequest, ExtractRequest, LoadHandle, OperationEvent,
        OperationProvider, OperationRequestId, TransferConflict,
    },
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};
use gtk::glib;
use std::{
    cell::RefCell,
    error::Error,
    fs,
    os::unix::fs::PermissionsExt,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};

fn run_compression(request: CompressRequest) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.compress(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Compressed { .. } | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }
    drop(operation);
    events.borrow().clone()
}

#[test]
fn compression_provider_rejects_escaping_archive_names() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    fs::create_dir(&destination)?;
    fs::write(&source, b"source")?;

    let events = run_compression(CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(&source)],
        destination: Location::local(&destination),
        archive_name: "../outside".to_owned(),
        conflict: TransferConflict::ReplaceExisting,
        format: ArchiveFormat::Zip,
        password: None,
    });

    assert!(matches!(events.as_slice(), [OperationEvent::Failed { .. }]));
    assert!(!root.path().join("outside.zip").exists());
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_conflict_choices_preserve_or_replace_the_destination() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    let archive = destination.join("existing.zip");
    fs::create_dir(&destination)?;
    fs::write(&source, b"replacement")?;
    fs::write(&archive, b"original")?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
    let request = |conflict| CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(&source)],
        destination: Location::local(&destination),
        archive_name: "existing".to_owned(),
        conflict,
        format: ArchiveFormat::Zip,
        password: None,
    };

    let refused = run_compression(request(TransferConflict::FailIfExists));
    assert!(
        refused
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    assert_eq!(fs::read(&archive)?, b"original");
    assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);

    let replaced = run_compression(request(TransferConflict::ReplaceExisting));
    assert!(
        replaced
            .iter()
            .any(|event| matches!(event, OperationEvent::Compressed { .. }))
    );
    let extracted = destination.join("extracted");
    fs::create_dir(&extracted)?;
    assert_eq!(
        extract_zip(&archive, &extracted)?,
        Some("source.txt".to_owned())
    );
    assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_failure_preserves_an_existing_archive() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let missing = root.path().join("missing.txt");
    let archive = destination.join("existing.zip");
    fs::create_dir(&destination)?;
    fs::write(&archive, b"original")?;

    let events = run_compression(CompressRequest {
        id: OperationRequestId(1),
        entries: vec![test_file_entry(&missing)],
        destination: Location::local(&destination),
        archive_name: "existing".to_owned(),
        conflict: TransferConflict::ReplaceExisting,
        format: ArchiveFormat::Zip,
        password: None,
    });

    assert!(
        events
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    assert_eq!(fs::read(&archive)?, b"original");
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn every_compression_format_commits_a_readable_archive() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    let mode_reference = root.path().join("mode-reference");
    fs::create_dir(&destination)?;
    fs::write(&source, b"contents")?;
    fs::File::create(&mode_reference)?;
    let expected_mode = fs::metadata(&mode_reference)?.permissions().mode() & 0o777;

    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::TarGz,
        ArchiveFormat::Tar,
    ] {
        let base = format!("archive-{}", format.extension().replace('.', "-"));
        let events = run_compression(CompressRequest {
            id: OperationRequestId(1),
            entries: vec![test_file_entry(&source)],
            destination: Location::local(&destination),
            archive_name: base.clone(),
            conflict: TransferConflict::FailIfExists,
            format,
            password: None,
        });
        assert!(
            events
                .iter()
                .any(|event| matches!(event, OperationEvent::Compressed { .. }))
        );
        let archive = destination.join(format!("{base}.{}", format.extension()));
        let extracted = destination.join(format!("extracted-{base}"));
        fs::create_dir(&extracted)?;
        match format {
            ArchiveFormat::Zip => {
                extract_zip(&archive, &extracted)?;
            }
            ArchiveFormat::SevenZ => {
                extract_7z_from_reader(
                    fs::File::open(&archive)?,
                    &extracted,
                    sevenz_rust2::Password::empty(),
                    &Arc::new(AtomicUsize::new(0)),
                    &never_cancelled(),
                )?;
            }
            ArchiveFormat::TarGz => {
                extract_tar(
                    &archive,
                    &extracted,
                    true,
                    &Arc::new(AtomicUsize::new(0)),
                    &never_cancelled(),
                )?;
            }
            ArchiveFormat::Tar => {
                extract_tar(
                    &archive,
                    &extracted,
                    false,
                    &Arc::new(AtomicUsize::new(0)),
                    &never_cancelled(),
                )?;
            }
        }
        assert_eq!(fs::read(extracted.join("source.txt"))?, b"contents");
        assert_eq!(
            fs::metadata(&archive)?.permissions().mode() & 0o777,
            expected_mode
        );
    }
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_reports_unsupported_7z_links_without_committing() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("file.txt"), b"contents")?;
    let link = source.join("link");
    std::os::unix::fs::symlink("file.txt", &link)?;
    for entry in [&source, &link] {
        for conflict in [
            TransferConflict::FailIfExists,
            TransferConflict::ReplaceExisting,
        ] {
            let destination = tempfile::tempdir()?;
            let archive = destination.path().join("archive.7z");
            if conflict == TransferConflict::ReplaceExisting {
                fs::write(&archive, b"original archive")?;
            }
            let events = run_compression(CompressRequest {
                id: OperationRequestId(1),
                entries: vec![test_file_entry(entry)],
                destination: Location::local(destination.path()),
                archive_name: "archive".to_owned(),
                conflict,
                format: ArchiveFormat::SevenZ,
                password: None,
            });
            assert!(events.iter().any(|event| matches!(event, OperationEvent::Failed { message, .. } if message.contains("does not support symbolic links") && message.contains("Use ZIP or TAR instead"))));
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, OperationEvent::Compressed { .. }))
            );
            if conflict == TransferConflict::ReplaceExisting {
                assert_eq!(fs::read(&archive)?, b"original archive");
            } else {
                assert!(!archive.exists());
            }
            assert!(compression_stages(destination.path())?.is_empty());
        }
    }
    Ok(())
}

#[test]
fn copy_with_big_buf_stops_when_cancelled() {
    let cancelled = AtomicBool::new(true);
    let mut destination = Vec::new();
    let error = copy_with_big_buf(&b"payload"[..], &mut destination, &cancelled)
        .expect_err("cancelled copy must stop");
    assert!(matches!(error, ArchiveError::Cancelled));
    assert!(destination.is_empty());
}

#[test]
fn cancelling_extraction_from_started_waits_for_the_worker_and_reports_pending_output()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.zip");
    write_zip_stored(
        &archive_path,
        &[("first.bin", b"early"), ("second.txt", b"late")],
    )?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = Rc::new(RefCell::new(None::<LoadHandle>));
    let cancel_on_started = operation.clone();
    let handle = LocalOperationProvider.extract(
        ExtractRequest {
            id: OperationRequestId(11),
            entry: test_file_entry(&archive_path),
            destination: Location::local(&destination),
            password: None,
        },
        Rc::new(move |event| {
            if matches!(event, OperationEvent::ArchiveStarted { .. }) {
                // Cancel before dispatching the worker, not after a main-context
                // iteration that may also finish extracting a small archive.
                drop(
                    cancel_on_started
                        .borrow_mut()
                        .take()
                        .expect("extraction handle"),
                );
            }
            emitted.borrow_mut().push(event);
        }),
    );
    operation.replace(Some(handle));
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Cancelled { .. }
                | OperationEvent::Extracted { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Cancelled { .. })),
        "expected cancellation after the worker stopped: {:?}",
        events.borrow()
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Extracted { .. }))
    );
    let result = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            OperationEvent::Cancelled { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("terminal cancellation result");
    assert!(
        result
            .affected_locations
            .contains(&Location::local(&destination))
    );
    assert!(result.completed.is_empty());
    assert!(result.failed.is_empty());
    assert_eq!(
        result.not_attempted,
        [
            Location::local(destination.join("first.bin")),
            Location::local(destination.join("second.txt")),
        ]
    );
    assert!(destination.read_dir()?.next().is_none());
    assert!(operation.borrow().is_none());
    Ok(())
}
