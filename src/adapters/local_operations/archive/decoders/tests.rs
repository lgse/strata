// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::fixtures::{
    always_cancelled, completed_extract, extract_zip, never_cancelled, write_7z, write_7z_entries,
    write_compression_fixture, write_tar, write_tar_entries, write_zip,
};
use super::{
    ArchiveError, ArchiveOutcome, extract_7z_from_reader, extract_tar, extract_zip_from_archive,
};
use crate::{model::Location, services::ArchiveFormat};
use std::{
    error::Error,
    fs,
    io::Cursor,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

fn decode_fixture(
    archive: &Path,
    destination: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let cancelled = never_cancelled();
    match format {
        ArchiveFormat::Zip => {
            let file = fs::File::open(archive).map_err(super::archive_failed)?;
            let mut archive = zip::ZipArchive::new(file).map_err(super::archive_failed)?;
            extract_zip_from_archive(&mut archive, destination, password, progress, &cancelled)
        }
        ArchiveFormat::SevenZ => extract_7z_from_reader(
            fs::File::open(archive).map_err(super::archive_failed)?,
            destination,
            password
                .map(sevenz_rust2::Password::from)
                .unwrap_or_default(),
            progress,
            &cancelled,
        ),
        ArchiveFormat::Tar | ArchiveFormat::TarGz => extract_tar(
            archive,
            destination,
            format == ArchiveFormat::TarGz,
            progress,
            &cancelled,
        ),
    }
}

#[test]
fn every_decoder_shares_nesting_conflicts_and_progress() -> Result<(), Box<dyn Error>> {
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::SevenZ, None),
        (ArchiveFormat::SevenZ, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("folder");
        fs::create_dir_all(source.join("nested"))?;
        fs::create_dir(source.join("empty"))?;
        fs::write(source.join("item.txt"), b"contents")?;
        fs::write(source.join("nested/zero.txt"), b"")?;
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, &[source], format, password)?;
        let destination = root.path().join("destination");
        fs::create_dir_all(destination.join("folder"))?;
        fs::write(destination.join("folder/keep.txt"), b"original")?;
        let progress = Arc::new(AtomicUsize::new(0));

        assert_eq!(
            completed_extract(decode_fixture(
                &archive,
                &destination,
                format,
                password,
                &progress
            )?)?,
            Some("folder (2)".to_owned()),
            "{format:?}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 5, "{format:?}");
        assert_eq!(fs::read(destination.join("folder/keep.txt"))?, b"original");
        assert_eq!(
            fs::read(destination.join("folder (2)/item.txt"))?,
            b"contents"
        );
        assert!(fs::read(destination.join("folder (2)/nested/zero.txt"))?.is_empty());
        assert!(destination.join("folder (2)/empty").is_dir());
    }
    Ok(())
}

#[test]
fn encrypted_decoders_fail_without_the_correct_password() -> Result<(), Box<dyn Error>> {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("file.txt");
        fs::write(&source, b"contents")?;
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, &[source], format, Some("test-password"))?;
        for password in [None, Some("wrong-password")] {
            let destination = tempfile::tempdir()?;
            assert!(
                matches!(
                    decode_fixture(
                        &archive,
                        destination.path(),
                        format,
                        password,
                        &Arc::new(AtomicUsize::new(0))
                    ),
                    Err(ArchiveError::Failed(_))
                ),
                "{format:?} accepted {password:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_archives_remain_failures_not_cancellations() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive");
    fs::write(&archive, b"not an archive")?;
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        let destination = tempfile::tempdir()?;
        let progress = Arc::new(AtomicUsize::new(0));
        assert!(
            matches!(
                decode_fixture(&archive, destination.path(), format, None, &progress),
                Err(ArchiveError::Failed(_))
            ),
            "{format:?}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        assert!(destination.path().read_dir()?.next().is_none());
    }
    Ok(())
}

#[test]
fn seven_z_extraction_preserves_all_file_contents() -> Result<(), Box<dyn Error>> {
    let entries = [
        ("folder/one.txt", b"first contents".as_slice()),
        ("folder/two.txt", b"second contents".as_slice()),
        ("folder/nested/three.txt", b"third contents".as_slice()),
    ];
    for solid in [true, false] {
        let root = tempfile::tempdir()?;
        let archive_path = root.path().join("files.7z");
        let destination = root.path().join("extracted");
        fs::create_dir(&destination)?;
        let mut writer = sevenz_rust2::ArchiveWriter::create(&archive_path)?;
        if solid {
            writer.push_archive_entries(
                entries
                    .iter()
                    .map(|(name, _)| sevenz_rust2::ArchiveEntry::new_file(name))
                    .collect(),
                entries
                    .iter()
                    .map(|(_, contents)| Cursor::new(*contents).into())
                    .collect(),
            )?;
        } else {
            for (name, contents) in &entries {
                writer.push_archive_entry(
                    sevenz_rust2::ArchiveEntry::new_file(name),
                    Some(Cursor::new(*contents)),
                )?;
            }
        }
        writer.finish()?;
        let reader = sevenz_rust2::ArchiveReader::new(
            fs::File::open(&archive_path)?,
            sevenz_rust2::Password::empty(),
        )?;
        assert_eq!(reader.archive().is_solid, solid);
        let progress = Arc::new(AtomicUsize::new(0));

        assert_eq!(
            completed_extract(extract_7z_from_reader(
                fs::File::open(&archive_path)?,
                &destination,
                sevenz_rust2::Password::empty(),
                &progress,
                &never_cancelled(),
            )?)?,
            Some("folder".to_owned())
        );
        for (name, contents) in &entries {
            assert_eq!(fs::read(destination.join(name))?, *contents);
        }
        assert_eq!(progress.load(Ordering::Relaxed), entries.len());
    }
    Ok(())
}

#[test]
fn seven_z_extraction_preserves_all_empty_files_and_directories() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive_path = root.path().join("empty-entries.7z");
    let destination = root.path().join("extracted");
    fs::create_dir(&destination)?;
    let mut writer = sevenz_rust2::ArchiveWriter::create(&archive_path)?;
    for entry in [
        sevenz_rust2::ArchiveEntry::new_directory("folder"),
        sevenz_rust2::ArchiveEntry::new_file("folder/one.txt"),
        sevenz_rust2::ArchiveEntry::new_directory("folder/empty"),
        sevenz_rust2::ArchiveEntry::new_file("folder/two.txt"),
    ] {
        writer.push_archive_entry::<Cursor<&[u8]>>(entry, None)?;
    }
    writer.finish()?;
    let progress = Arc::new(AtomicUsize::new(0));

    assert_eq!(
        completed_extract(extract_7z_from_reader(
            fs::File::open(&archive_path)?,
            &destination,
            sevenz_rust2::Password::empty(),
            &progress,
            &never_cancelled(),
        )?)?,
        Some("folder".to_owned())
    );
    assert!(destination.join("folder/empty").is_dir());
    for name in ["folder/one.txt", "folder/two.txt"] {
        assert!(fs::read(destination.join(name))?.is_empty());
    }
    assert_eq!(progress.load(Ordering::Relaxed), 4);
    Ok(())
}

#[test]
fn tar_extraction_skips_root_directories_and_preserves_contents() -> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        for root_entry in [None, Some("."), Some("./")] {
            let root = tempfile::tempdir()?;
            let destination = root.path().join("destination");
            fs::create_dir_all(destination.join("folder"))?;
            fs::write(destination.join("folder/keep.txt"), b"keep")?;
            let archive = root.path().join("content.tar");
            let mut entries = Vec::new();
            if let Some(name) = root_entry {
                entries.push((tar::EntryType::Directory, name, b"".as_slice()));
            }
            entries.extend([
                (tar::EntryType::Directory, "./folder/", b"".as_slice()),
                (tar::EntryType::Regular, "./folder/item.txt", b"contents"),
                (tar::EntryType::Regular, "./empty.txt", b""),
            ]);
            write_tar_entries(&archive, &entries, gzip)?;
            let progress = Arc::new(AtomicUsize::new(0));
            assert_eq!(
                completed_extract(extract_tar(
                    &archive,
                    &destination,
                    gzip,
                    &progress,
                    &never_cancelled(),
                )?)?,
                Some("folder (2)".to_owned()),
            );
            assert_eq!(progress.load(Ordering::Relaxed), 3);
            assert_eq!(
                fs::read(destination.join("folder (2)/item.txt"))?,
                b"contents"
            );
            assert_eq!(fs::read(destination.join("folder/keep.txt"))?, b"keep");
            assert_eq!(fs::metadata(destination.join("empty.txt"))?.len(), 0);
            assert_eq!(fs::read_dir(&destination)?.count(), 3);
        }
    }
    Ok(())
}

#[test]
fn tar_extraction_root_only_completes_without_a_name_and_respects_cancellation()
-> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let archive = root.path().join("content.tar");
        write_tar_entries(&archive, &[(tar::EntryType::Directory, "./", b"")], gzip)?;
        let progress = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            completed_extract(extract_tar(
                &archive,
                &destination,
                gzip,
                &progress,
                &never_cancelled(),
            )?)?,
            None,
        );
        assert!(matches!(
            extract_tar(&archive, &destination, gzip, &progress, &always_cancelled())?,
            ArchiveOutcome::Cancelled { completed, failed, not_attempted }
                if completed.is_empty() && failed.is_empty() && not_attempted.is_empty()
        ));
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        assert!(fs::read_dir(&destination)?.next().is_none());
    }
    Ok(())
}

#[test]
fn tar_extraction_rejects_empty_paths_and_root_file_entries() -> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        for (entry_type, name) in [
            (tar::EntryType::Directory, ""),
            (tar::EntryType::Directory, "/"),
            (tar::EntryType::Regular, ""),
            (tar::EntryType::Regular, "."),
            (tar::EntryType::Regular, "./"),
            (tar::EntryType::Regular, "././"),
        ] {
            let root = tempfile::tempdir()?;
            let destination = root.path().join("destination");
            fs::create_dir(&destination)?;
            let archive = root.path().join("content.tar");
            write_tar_entries(&archive, &[(entry_type, name, b"")], gzip)?;
            let progress = Arc::new(AtomicUsize::new(0));
            assert!(
                matches!(
                    extract_tar(&archive, &destination, gzip, &progress, &never_cancelled()),
                    Err(ArchiveError::Failed(_))
                ),
                "accepted {entry_type:?} {name:?}, gzip={gzip}"
            );
            assert_eq!(progress.load(Ordering::Relaxed), 0);
            assert!(fs::read_dir(&destination)?.next().is_none());
        }
    }
    Ok(())
}

#[test]
fn every_archive_format_rejects_parent_traversal() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let zip_path = root.path().join("malicious.zip");
    let tar_path = root.path().join("malicious.tar");
    let tar_gz_path = root.path().join("malicious.tar.gz");
    let seven_z_path = root.path().join("malicious.7z");
    write_zip(&zip_path, &[("../zip-marker", b"escaped")])?;
    write_tar(&tar_path, "../tar-marker", b"escaped", false)?;
    write_tar(&tar_gz_path, "../tar-gz-marker", b"escaped", true)?;
    write_7z(&seven_z_path, "../seven-z-marker", b"escaped")?;

    assert!(extract_zip(&zip_path, &destination).is_err());
    assert!(
        extract_tar(
            &tar_path,
            &destination,
            false,
            &Arc::new(AtomicUsize::new(0)),
            &never_cancelled(),
        )
        .is_err()
    );
    assert!(
        extract_tar(
            &tar_gz_path,
            &destination,
            true,
            &Arc::new(AtomicUsize::new(0)),
            &never_cancelled(),
        )
        .is_err()
    );
    assert!(
        extract_7z_from_reader(
            fs::File::open(&seven_z_path)?,
            &destination,
            sevenz_rust2::Password::empty(),
            &Arc::new(AtomicUsize::new(0)),
            &never_cancelled(),
        )
        .is_err()
    );

    for marker in [
        "zip-marker",
        "tar-marker",
        "tar-gz-marker",
        "seven-z-marker",
    ] {
        assert!(!root.path().join(marker).exists(), "created {marker}");
    }
    Ok(())
}

#[test]
fn extraction_rejects_final_and_intermediate_symlinks() -> Result<(), Box<dyn Error>> {
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        for name in ["dangling", "redirect/marker"] {
            let root = tempfile::tempdir()?;
            let destination = root.path().join("destination");
            let external = root.path().join("external");
            fs::create_dir(&destination)?;
            fs::create_dir(&external)?;
            std::os::unix::fs::symlink(root.path().join("missing"), destination.join("dangling"))?;
            std::os::unix::fs::symlink(&external, destination.join("redirect"))?;
            let archive = root.path().join("archive");
            match format {
                ArchiveFormat::Zip => write_zip(&archive, &[(name, b"escaped")])?,
                ArchiveFormat::SevenZ => write_7z(&archive, name, b"escaped")?,
                ArchiveFormat::Tar | ArchiveFormat::TarGz => {
                    write_tar(&archive, name, b"escaped", format == ArchiveFormat::TarGz)?;
                }
            }
            assert!(
                decode_fixture(
                    &archive,
                    &destination,
                    format,
                    None,
                    &Arc::new(AtomicUsize::new(0))
                )
                .is_err(),
                "{format:?} accepted {name}"
            );
            assert!(!root.path().join("missing").exists());
            assert!(!external.join("marker").exists());
        }
    }
    Ok(())
}

#[test]
fn extraction_supports_nesting_and_regular_conflicts() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    fs::write(destination.join("report.txt"), b"original")?;
    fs::create_dir(destination.join("existing"))?;
    fs::write(destination.join("existing/old.txt"), b"old")?;
    let archive_path = root.path().join("content.zip");
    write_zip(
        &archive_path,
        &[
            ("folder/nested/item.txt", b"nested"),
            ("report.txt", b"replacement"),
            ("existing/new.txt", b"new"),
        ],
    )?;

    assert_eq!(
        extract_zip(&archive_path, &destination)?.as_deref(),
        Some("folder")
    );
    assert_eq!(
        fs::read(destination.join("folder/nested/item.txt"))?,
        b"nested"
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"original");
    assert_eq!(
        fs::read(destination.join("report (2).txt"))?,
        b"replacement"
    );
    assert_eq!(fs::read(destination.join("existing/old.txt"))?, b"old");
    assert_eq!(fs::read(destination.join("existing (2)/new.txt"))?, b"new");
    Ok(())
}

#[test]
fn zip_extraction_stops_and_drops_incomplete_output_when_cancelled() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.zip");
    write_zip(
        &archive_path,
        &[("first.bin", b"early"), ("second.txt", b"late")],
    )?;

    let file = fs::File::open(&archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let outcome = extract_zip_from_archive(
        &mut archive,
        &destination,
        None,
        &Arc::new(AtomicUsize::new(0)),
        &Arc::new(AtomicBool::new(true)),
    )?;

    match outcome {
        ArchiveOutcome::Cancelled {
            completed,
            failed,
            not_attempted,
        } => {
            assert!(completed.is_empty());
            assert!(failed.is_empty());
            assert_eq!(not_attempted.len(), 2);
        }
        ArchiveOutcome::Completed(_) => panic!("extraction continued after cancellation"),
    }
    assert!(destination.read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn tar_extraction_stops_without_scanning_remaining_entries() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.tar");
    write_tar_entries(
        &archive_path,
        &[
            (tar::EntryType::Regular, "first.bin", b"early"),
            (tar::EntryType::Regular, "second.txt", b"late"),
        ],
        false,
    )?;

    let outcome = extract_tar(
        &archive_path,
        &destination,
        false,
        &Arc::new(AtomicUsize::new(0)),
        &always_cancelled(),
    )?;

    match outcome {
        ArchiveOutcome::Cancelled {
            completed,
            failed,
            not_attempted,
        } => {
            assert!(completed.is_empty());
            assert!(failed.is_empty());
            assert_eq!(
                not_attempted,
                [Location::local(destination.join("first.bin"))]
            );
        }
        ArchiveOutcome::Completed(_) => panic!("extraction continued after cancellation"),
    }
    assert!(destination.read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn sevenz_extraction_reports_remaining_entries_when_cancelled() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive_path = root.path().join("content.7z");
    write_7z_entries(
        &archive_path,
        &[("first.bin", b"early"), ("second.txt", b"late")],
    )?;

    let outcome = extract_7z_from_reader(
        fs::File::open(&archive_path)?,
        &destination,
        sevenz_rust2::Password::empty(),
        &Arc::new(AtomicUsize::new(0)),
        &always_cancelled(),
    )?;

    match outcome {
        ArchiveOutcome::Cancelled {
            completed,
            failed,
            not_attempted,
        } => {
            assert!(completed.is_empty());
            assert!(failed.is_empty());
            assert_eq!(
                not_attempted,
                [
                    Location::local(destination.join("first.bin")),
                    Location::local(destination.join("second.txt")),
                ]
            );
        }
        ArchiveOutcome::Completed(_) => panic!("extraction continued after cancellation"),
    }
    assert!(destination.read_dir()?.next().is_none());
    Ok(())
}
