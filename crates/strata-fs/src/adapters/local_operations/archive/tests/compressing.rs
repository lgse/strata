// SPDX-License-Identifier: MIT

//! Compressing through the operation provider, and the buffered copy under it.

use super::*;

#[test]
fn compression_provider_rejects_escaping_archive_names() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    fs::create_dir(&destination)?;
    fs::write(&source, b"source")?;

    let events = run_compression(zip_request(
        &source,
        &destination,
        "../outside",
        TransferConflict::ReplaceExisting,
    ));

    assert!(matches!(events.as_slice(), [OperationEvent::Failed { .. }]));
    assert!(!root.path().join("outside.zip").exists());
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

#[test]
fn an_archive_name_the_location_would_store_otherwise_is_refused_before_anything_is_written()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows: [(Preset, &str, Option<String>); 2] = [
        (
            Quirks::vfat,
            "nul",
            Some(
                rust_i18n::t!(
                    "“%{name}” is a reserved name in this location",
                    name = "nul"
                )
                .into_owned(),
            ),
        ),
        (Quirks::ext4, "a:b", None),
    ];
    for (quirks, archive_name, refusal) in rows {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        let source = root.path().join("source.txt");
        fs::create_dir(&destination)?;
        fs::write(&source, b"source")?;
        let _scope = FsQuirks::install(&destination, quirks());

        let events = run_compression(zip_request(
            &source,
            &destination,
            archive_name,
            TransferConflict::FailIfExists,
        ));

        match refusal {
            Some(refusal) => {
                assert!(
                    matches!(events.last(), Some(OperationEvent::Failed { message, .. }) if *message == refusal),
                    "{archive_name}: {events:?}"
                );
                assert!(entry_names(&destination)?.is_empty(), "{archive_name}");
            }
            None => {
                assert!(
                    matches!(events.last(), Some(OperationEvent::Compressed { .. })),
                    "{archive_name}: {events:?}"
                );
                assert_eq!(entry_names(&destination)?, [format!("{archive_name}.zip")]);
            }
        }
    }
    Ok(())
}

fn expect_original_mode(archive: &Path) -> Result<(), Box<dyn Error>> {
    assert_eq!(fs::metadata(archive)?.permissions().mode() & 0o777, 0o640);
    Ok(())
}

#[test]
fn compression_conflict_choices_preserve_or_replace_the_destination() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempdir_on_home_device()?;
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    let archive = destination.join("existing.zip");
    let _trash = HomeTrashGuard::new(&archive);
    fs::create_dir(&destination)?;
    fs::write(&source, b"replacement")?;
    fs::write(&archive, b"original")?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
    let request = |conflict| zip_request(&source, &destination, "existing", conflict);

    let refused = run_compression(request(TransferConflict::FailIfExists));
    assert!(
        refused
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    assert_eq!(fs::read(&archive)?, b"original");
    expect_original_mode(&archive)?;

    for suffix in [1, 2] {
        let kept = run_compression(request(TransferConflict::KeepBoth));
        let expected_name = format!("existing ({suffix}).zip");
        assert!(kept.iter().any(|event| matches!(event,
            OperationEvent::Compressed { archive_name, archive, original: None, .. }
                if archive_name == &expected_name
                    && archive == &Location::local(destination.join(&expected_name))
        )));
        let extracted = destination.join(format!("kept-{suffix}"));
        fs::create_dir(&extracted)?;
        extract_zip(&destination.join(expected_name), &extracted)?;
        expect_contents(
            &[
                (extracted.join("source.txt"), b"replacement"),
                (archive.clone(), b"original"),
            ],
            "kept both",
        )?;
        expect_original_mode(&archive)?;
    }

    let replaced = run_compression(request(TransferConflict::ReplaceExisting));
    assert!(replaced.iter().any(|event| matches!(
        event,
        OperationEvent::Compressed {
            original: Some(_),
            ..
        }
    )));
    let extracted = destination.join("extracted");
    fs::create_dir(&extracted)?;
    assert_eq!(
        extract_zip(&archive, &extracted)?,
        Some("source.txt".to_owned())
    );
    expect_original_mode(&archive)?;
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

#[test]
fn compression_failure_preserves_an_existing_archive() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    let missing = root.path().join("missing.txt");
    let archive = destination.join("existing.zip");
    fs::create_dir(&destination)?;
    fs::write(&archive, b"original")?;

    let events = run_compression(zip_request(
        &missing,
        &destination,
        "existing",
        TransferConflict::ReplaceExisting,
    ));

    assert!(
        events
            .iter()
            .any(|event| matches!(event, OperationEvent::Failed { .. }))
    );
    assert_eq!(fs::read(&archive)?, b"original");
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

fn kept_items(events: &[OperationEvent], id: OperationRequestId) -> Vec<KeptItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { request_id, report } if *request_id == id => {
                Some(report.kept.clone())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// Checks the one kept item is the original at `kept_original`, with an error that names no stage.
fn expect_kept_original(kept: &[KeptItem], kept_original: &Path) -> Result<(), Box<dyn Error>> {
    let [
        KeptItem {
            location,
            cause: KeptCause::OriginalNotRemoved(error),
        },
    ] = kept
    else {
        return Err(format!("the original is reported as kept: {kept:?}").into());
    };
    assert_eq!(location, &Location::local(kept_original));
    assert!(!error.contains(".strata-"), "{error}");
    Ok(())
}

#[test]
fn an_archive_that_replaced_an_original_it_cannot_remove_completes_and_reports_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    if base_places_files_by_linking(root.path())? {
        eprintln!("skipped on a base that places files by linking them");
        return Ok(());
    }
    let destination = root.path().join("destination");
    let source = root.path().join("source.txt");
    fs::create_dir(&destination)?;
    fs::write(&source, b"replacement")?;
    fs::write(destination.join("existing.zip"), b"original")?;
    let _scope = FsQuirks::install(
        &destination,
        Quirks {
            trash: Trash::Unsupported,
            fail: vec![(Op::Unlink, "existing.zip", 0, Errno::ACCESS)],
            ..Quirks::ext4()
        },
    );

    let events = run_compression(CompressRequest {
        id: OperationRequestId(4),
        ..zip_request(
            &source,
            &destination,
            "existing",
            TransferConflict::ReplaceExisting,
        )
    });

    assert!(
        matches!(
            events.last(),
            Some(OperationEvent::Compressed {
                archive_name,
                original: Some(ReplacedOriginal::Deleted),
                ..
            }) if archive_name == "existing.zip"
        ),
        "{events:?}"
    );
    let names = entry_names(&destination)?;
    let [originals, published] = names.as_slice() else {
        return Err(format!("the kept original and the archive: {names:?}").into());
    };
    assert!(originals.starts_with(".strata-originals-"), "{names:?}");
    assert_eq!(published, "existing.zip");
    let kept_original = destination.join(originals).join("existing.zip");
    assert_eq!(fs::read(&kept_original)?, b"original");
    let extracted = root.path().join("extracted");
    fs::create_dir(&extracted)?;
    assert_eq!(
        extract_zip(&destination.join("existing.zip"), &extracted)?,
        Some("source.txt".to_owned())
    );
    expect_kept_original(&kept_items(&events, OperationRequestId(4)), &kept_original)
}

/// Extracts `archive` into `extracted` with the decoder for `format`.
fn decode_into(
    archive: &Path,
    extracted: &Path,
    format: ArchiveFormat,
) -> Result<(), Box<dyn Error>> {
    let progress = Arc::new(AtomicUsize::new(0));
    let mut report = TransferReport::default();
    match format {
        ArchiveFormat::Zip => {
            extract_zip(archive, extracted)?;
        }
        ArchiveFormat::SevenZ => {
            extract_7z_from_reader(
                fs::File::open(archive)?,
                extracted,
                "archive",
                sevenz_rust2::Password::empty(),
                &progress,
                &never_cancelled(),
                &mut report,
            )?;
        }
        ArchiveFormat::TarGz | ArchiveFormat::Tar => {
            extract_tar(
                archive,
                extracted,
                "archive",
                format == ArchiveFormat::TarGz,
                &progress,
                &never_cancelled(),
                &mut report,
            )?;
        }
        ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
    }
    Ok(())
}

#[test]
fn every_compression_format_commits_a_readable_archive() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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
            format,
            ..zip_request(&source, &destination, &base, TransferConflict::FailIfExists)
        });
        assert!(
            events
                .iter()
                .any(|event| matches!(event, OperationEvent::Compressed { .. }))
        );
        let archive = destination.join(format!("{base}.{}", format.extension()));
        let extracted = destination.join(format!("extracted-{base}"));
        fs::create_dir(&extracted)?;
        decode_into(&archive, &extracted, format)?;
        assert_eq!(fs::read(extracted.join("source.txt"))?, b"contents");
        assert_eq!(
            fs::metadata(&archive)?.permissions().mode() & 0o777,
            expected_mode
        );
    }
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

fn expect_unsupported_7z_links(
    entry: &Path,
    conflict: TransferConflict,
) -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    let archive = destination.path().join("archive.7z");
    let replacing = conflict == TransferConflict::ReplaceExisting;
    if replacing {
        fs::write(&archive, b"original archive")?;
    }
    let events = run_compression(CompressRequest {
        format: ArchiveFormat::SevenZ,
        ..zip_request(entry, destination.path(), "archive", conflict)
    });
    assert!(events.iter().any(|event| matches!(event, OperationEvent::Failed { message, .. } if message.contains("does not support symbolic links") && message.contains("Use ZIP or TAR instead"))));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, OperationEvent::Compressed { .. }))
    );
    if replacing {
        assert_eq!(fs::read(&archive)?, b"original archive");
    } else {
        assert!(!archive.exists());
    }
    assert!(stages(destination.path(), COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

#[test]
fn compression_reports_unsupported_7z_links_without_committing() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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
            expect_unsupported_7z_links(entry, conflict)?;
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
fn copy_with_big_buf_reports_os_write_failures_without_the_errno() {
    struct FullDisk;
    impl std::io::Write for FullDisk {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from_raw_os_error(libc::EFBIG))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let error = copy_with_big_buf(&b"payload"[..], &mut FullDisk, &AtomicBool::new(false))
        .expect_err("failed write must stop the copy");
    assert_eq!(
        error,
        ArchiveError::Failed("The file is too large for this file system".to_owned())
    );
}
