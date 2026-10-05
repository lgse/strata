// SPDX-License-Identifier: MIT

use super::super::fixtures::{
    FixtureMember, always_cancelled, completed_extract, corrupt_gzip_trailer, expected_mode,
    extract_zip, never_cancelled, patch_zip_external_attributes, patch_zip_uncompressed_size,
    split_into_gzip_members, write_7z, write_7z_entries, write_compression_fixture, write_members,
    write_tar, write_tar_entries, write_zip, zip_extended_timestamp,
};
use super::{
    ArchiveError, ArchiveOutcome, extract_7z_from_reader, extract_tar, extract_zip_from_archive,
};
use crate::{model::Location, services::ArchiveFormat};
use gtk::glib;
use std::{
    error::Error,
    ffi::OsStr,
    fs,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
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
    let name = archive
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("archive");
    match format {
        ArchiveFormat::Zip => {
            let file = fs::File::open(archive).map_err(super::archive_failed)?;
            let mut archive = zip::ZipArchive::new(file).map_err(super::zip_error)?;
            extract_zip_from_archive(
                &mut archive,
                destination,
                name,
                password,
                progress,
                &cancelled,
            )
        }
        ArchiveFormat::SevenZ => extract_7z_from_reader(
            fs::File::open(archive).map_err(super::archive_failed)?,
            destination,
            name,
            password
                .map(sevenz_rust2::Password::from)
                .unwrap_or_default(),
            progress,
            &cancelled,
        ),
        ArchiveFormat::Tar | ArchiveFormat::TarGz => extract_tar(
            archive,
            destination,
            name,
            format == ArchiveFormat::TarGz,
            progress,
            &cancelled,
        ),
        ArchiveFormat::Rar => unreachable!("no fixture test exercises RAR through this helper"),
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
                "archive",
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
            "archive",
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
                    "content.tar",
                    gzip,
                    &progress,
                    &never_cancelled(),
                )?)?,
                Some("content".to_owned()),
            );
            assert_eq!(progress.load(Ordering::Relaxed), 3);
            assert_eq!(
                fs::read(destination.join("content/folder/item.txt"))?,
                b"contents"
            );
            assert_eq!(fs::read(destination.join("folder/keep.txt"))?, b"keep");
            assert_eq!(
                fs::metadata(destination.join("content/empty.txt"))?.len(),
                0
            );
            assert_eq!(fs::read_dir(&destination)?.count(), 2);
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
                "archive",
                gzip,
                &progress,
                &never_cancelled(),
            )?)?,
            None,
        );
        assert!(matches!(
            extract_tar(&archive, &destination, "archive", gzip, &progress, &always_cancelled())?,
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
                    extract_tar(
                        &archive,
                        &destination,
                        "archive",
                        gzip,
                        &progress,
                        &never_cancelled()
                    ),
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
fn every_archive_format_sanitizes_parent_traversal_without_stopping() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive");
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        let entries = [
            ("../escaped.txt", &b"escaped"[..]),
            ("after.txt", &b"after"[..]),
        ];
        match format {
            ArchiveFormat::Zip => write_zip(&archive, &entries)?,
            ArchiveFormat::SevenZ => write_7z_entries(&archive, &entries)?,
            ArchiveFormat::Tar | ArchiveFormat::TarGz => write_tar_entries(
                &archive,
                &entries.map(|(name, contents)| (tar::EntryType::Regular, name, contents)),
                format == ArchiveFormat::TarGz,
            )?,
            ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
        }
        let destination = tempfile::tempdir_in(root.path())?;
        let progress = Arc::new(AtomicUsize::new(0));

        assert_eq!(
            completed_extract(decode_fixture(
                &archive,
                destination.path(),
                format,
                None,
                &progress,
            )?)?,
            Some("archive".to_owned()),
            "{format:?}"
        );
        assert_eq!(
            fs::read(destination.path().join("archive/escaped.txt"))?,
            b"escaped"
        );
        assert_eq!(
            fs::read(destination.path().join("archive/after.txt"))?,
            b"after"
        );
        assert!(!root.path().join("escaped.txt").exists());
        assert_eq!(progress.load(Ordering::Relaxed), 2);
    }
    Ok(())
}

#[test]
fn destination_symlinks_are_never_followed_and_colliding_roots_are_suffixed()
-> Result<(), Box<dyn Error>> {
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        for (name, published, written) in [
            ("dangling", "dangling (2)", "dangling (2)"),
            ("redirect/marker", "redirect (2)", "redirect (2)/marker"),
        ] {
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
                ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
            }
            assert_eq!(
                completed_extract(decode_fixture(
                    &archive,
                    &destination,
                    format,
                    None,
                    &Arc::new(AtomicUsize::new(0))
                )?)?,
                Some(published.to_owned()),
                "{format:?} {name}"
            );
            assert_eq!(fs::read(destination.join(written))?, b"escaped");
            assert!(!root.path().join("missing").exists());
            assert!(!external.join("marker").exists());
            assert_eq!(
                fs::read_link(destination.join("dangling"))?,
                root.path().join("missing")
            );
            assert_eq!(fs::read_link(destination.join("redirect"))?, external);
        }
    }
    Ok(())
}

#[test]
fn multi_root_zip_keeps_member_names_inside_the_fresh_folder() -> Result<(), Box<dyn Error>> {
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
        extract_zip(&archive_path, &destination)?,
        Some("content".to_owned())
    );
    assert_eq!(
        fs::read(destination.join("content/folder/nested/item.txt"))?,
        b"nested"
    );
    assert_eq!(
        fs::read(destination.join("content/report.txt"))?,
        b"replacement"
    );
    assert_eq!(
        fs::read(destination.join("content/existing/new.txt"))?,
        b"new"
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"original");
    assert_eq!(fs::read(destination.join("existing/old.txt"))?, b"old");
    assert_eq!(destination.join("existing").read_dir()?.count(), 1);
    assert_eq!(fs::read_dir(&destination)?.count(), 3);
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
        "archive",
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
        "archive",
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

struct CancelAfterMembers<'a> {
    file: fs::File,
    progress: &'a AtomicUsize,
    cancelled: &'a AtomicBool,
    after: usize,
}

impl Read for CancelAfterMembers<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.progress.load(Ordering::Relaxed) >= self.after {
            self.cancelled.store(true, Ordering::Relaxed);
        }
        self.file.read(buffer)
    }
}

impl Seek for CancelAfterMembers<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

fn write_mixed_7z(path: &Path, solid: bool) -> Result<(), Box<dyn Error>> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path)?;
    writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    for entry in [
        sevenz_rust2::ArchiveEntry::new_directory("folder"),
        sevenz_rust2::ArchiveEntry::new_file("folder/same.txt"),
    ] {
        writer.push_archive_entry::<Cursor<&[u8]>>(entry, None)?;
    }
    let entries = [
        ("folder/same.txt", b"one".as_slice()),
        ("folder/same.txt", b"two".as_slice()),
        ("folder/later.txt", b"later".as_slice()),
    ];
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
        for (name, contents) in entries {
            writer.push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file(name),
                Some(Cursor::new(contents)),
            )?;
        }
    }
    for entry in [
        sevenz_rust2::ArchiveEntry::new_directory("folder/empty"),
        sevenz_rust2::ArchiveEntry::new_file("folder/zero.txt"),
    ] {
        writer.push_archive_entry::<Cursor<&[u8]>>(entry, None)?;
    }
    writer.finish()?;
    Ok(())
}

#[test]
fn sevenz_cancellation_keeps_streamless_and_duplicate_members_pending() -> Result<(), Box<dyn Error>>
{
    for solid in [false, true] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("mixed.7z");
        write_mixed_7z(&archive, solid)?;
        let destination = tempfile::tempdir()?;
        let progress = Arc::new(AtomicUsize::new(0));
        let outcome = extract_7z_from_reader(
            fs::File::open(&archive)?,
            destination.path(),
            "archive",
            sevenz_rust2::Password::empty(),
            &progress,
            &always_cancelled(),
        )?;
        let ArchiveOutcome::Cancelled {
            completed,
            failed,
            not_attempted,
        } = outcome
        else {
            panic!("expected cancellation before the first callback");
        };
        assert!(completed.is_empty());
        assert!(failed.is_empty());
        assert_eq!(
            not_attempted,
            [
                "folder",
                "folder/same.txt",
                "folder/same.txt",
                "folder/same.txt",
                "folder/later.txt",
                "folder/empty",
                "folder/zero.txt"
            ]
            .map(|name| Location::local(destination.path().join(name))),
            "solid={solid}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        assert!(destination.path().read_dir()?.next().is_none());
    }
    Ok(())
}

#[test]
fn sevenz_mid_copy_cancellation_tracks_header_identity_and_destination_renames()
-> Result<(), Box<dyn Error>> {
    for solid in [false, true] {
        for after in [1, 2] {
            let root = tempfile::tempdir()?;
            let archive = root.path().join("mixed.7z");
            write_mixed_7z(&archive, solid)?;
            let destination = tempfile::tempdir()?;
            fs::create_dir(destination.path().join("folder"))?;
            fs::write(destination.path().join("folder/keep.txt"), b"original")?;
            let progress = Arc::new(AtomicUsize::new(0));
            let cancelled = AtomicBool::new(false);
            let reader = CancelAfterMembers {
                file: fs::File::open(&archive)?,
                progress: &progress,
                cancelled: &cancelled,
                after,
            };
            let outcome = extract_7z_from_reader(
                reader,
                destination.path(),
                "mixed.7z",
                sevenz_rust2::Password::empty(),
                &progress,
                &cancelled,
            )?;
            let ArchiveOutcome::Cancelled {
                completed,
                failed,
                not_attempted,
            } = outcome
            else {
                panic!("expected cancellation after {after} members, solid={solid}");
            };
            let location = |name| Location::local(destination.path().join(name));
            let completed_names = ["mixed/folder/same.txt", "mixed/folder/same (2).txt"];
            assert_eq!(
                completed,
                completed_names[..after]
                    .iter()
                    .map(location)
                    .collect::<Vec<_>>()
            );
            assert!(failed.is_empty());
            let interrupted = if after == 1 {
                "mixed/folder/same (2).txt"
            } else {
                "mixed/folder/later.txt"
            };
            let mut pending_names = vec![interrupted, "mixed/folder", "mixed/folder/same.txt"];
            if after == 1 {
                pending_names.push("mixed/folder/later.txt");
            }
            pending_names.extend(["mixed/folder/empty", "mixed/folder/zero.txt"]);
            assert_eq!(
                not_attempted,
                pending_names.iter().map(location).collect::<Vec<_>>(),
                "after={after}, solid={solid}"
            );
            assert_eq!(completed.len() + not_attempted.len(), 7);
            assert_eq!(progress.load(Ordering::Relaxed), after);
            assert_eq!(
                fs::read(destination.path().join("folder/keep.txt"))?,
                b"original"
            );
            assert_eq!(
                fs::read(destination.path().join(completed_names[0]))?,
                b"one"
            );
            if after == 2 {
                assert_eq!(
                    fs::read(destination.path().join(completed_names[1]))?,
                    b"two"
                );
            }
            assert_eq!(
                destination.path().join("mixed/folder").read_dir()?.count(),
                after
            );
            assert_eq!(destination.path().join("folder").read_dir()?.count(), 1);
            assert!(!destination.path().join(interrupted).exists());
        }
    }
    Ok(())
}

#[test]
fn pending_unsafe_names_are_sanitized_by_every_decoder() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive");
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        match format {
            ArchiveFormat::Zip => write_zip(&archive, &[("../outside", b"contents")])?,
            ArchiveFormat::SevenZ => write_7z(&archive, "../outside", b"contents")?,
            ArchiveFormat::Tar | ArchiveFormat::TarGz => write_tar(
                &archive,
                "../outside",
                b"contents",
                format == ArchiveFormat::TarGz,
            )?,
            ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
        }
        let destination = tempfile::tempdir()?;
        let cancelled = always_cancelled();
        let progress = Arc::new(AtomicUsize::new(0));
        let outcome = match format {
            ArchiveFormat::Zip => {
                let mut archive = zip::ZipArchive::new(fs::File::open(&archive)?)?;
                extract_zip_from_archive(
                    &mut archive,
                    destination.path(),
                    "archive",
                    None,
                    &progress,
                    &cancelled,
                )?
            }
            ArchiveFormat::SevenZ => extract_7z_from_reader(
                fs::File::open(&archive)?,
                destination.path(),
                "archive",
                sevenz_rust2::Password::empty(),
                &progress,
                &cancelled,
            )?,
            ArchiveFormat::Tar | ArchiveFormat::TarGz => extract_tar(
                &archive,
                destination.path(),
                "archive",
                format == ArchiveFormat::TarGz,
                &progress,
                &cancelled,
            )?,
            ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
        };
        assert!(
            matches!(outcome, ArchiveOutcome::Cancelled { completed, failed, not_attempted }
            if completed.is_empty() && failed.is_empty()
                && not_attempted == [Location::local(destination.path().join("outside"))]),
            "{format:?}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        assert!(destination.path().read_dir()?.next().is_none());
    }
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
        "archive",
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

#[test]
fn zip_member_lying_about_its_size_is_refused_before_it_expands() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("bomb.zip");
    write_zip(&archive, &[("zeros.bin", &vec![0u8; 8 << 20])])?;
    patch_zip_uncompressed_size(&archive, 16)?;
    assert!(
        fs::metadata(&archive)?.len() < 64 << 10,
        "fixture should be a small archive that expands to 8 MiB"
    );
    let destination = tempfile::tempdir()?;

    let error = extract_zip(&archive, destination.path())
        .expect_err("a member exceeding its declared size should fail");

    assert!(
        error.contains("`zeros.bin` declared 16 bytes but produced more"),
        "{error}"
    );
    assert!(
        destination.path().read_dir()?.next().is_none(),
        "the partial member should be removed"
    );
    Ok(())
}

#[test]
fn zip_member_declaring_more_than_it_contains_is_refused() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("short.zip");
    write_zip(&archive, &[("note.txt", b"hello")])?;
    patch_zip_uncompressed_size(&archive, 1000)?;
    let destination = tempfile::tempdir()?;

    let error = extract_zip(&archive, destination.path())
        .expect_err("a member shorter than its declared size should fail");

    assert!(
        error.contains("`note.txt` declared 1000 bytes but produced 5 bytes"),
        "{error}"
    );
    assert!(
        destination.path().read_dir()?.next().is_none(),
        "the truncated member should be removed"
    );
    Ok(())
}

#[test]
fn highly_compressible_archives_extract_in_every_format() -> Result<(), Box<dyn Error>> {
    const SIZE: u64 = 16 << 20;
    let zeros = vec![0u8; SIZE as usize];
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::TarGz,
    ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("zeros.archive");
        match format {
            ArchiveFormat::Zip => write_zip(&archive, &[("zeros.bin", &zeros)])?,
            ArchiveFormat::SevenZ => write_7z(&archive, "zeros.bin", &zeros)?,
            ArchiveFormat::Tar | ArchiveFormat::TarGz => {
                write_tar(&archive, "zeros.bin", &zeros, true)?;
            }
            ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
        }
        let ratio = SIZE / fs::metadata(&archive)?.len();
        assert!(
            ratio > 100,
            "{format:?} fixture should compress well, got {ratio}:1"
        );
        let destination = tempfile::tempdir()?;
        let progress = Arc::new(AtomicUsize::new(0));

        let first_name = completed_extract(decode_fixture(
            &archive,
            destination.path(),
            format,
            None,
            &progress,
        )?)?;

        assert_eq!(first_name.as_deref(), Some("zeros.bin"), "{format:?}");
        assert_eq!(
            fs::metadata(destination.path().join("zeros.bin"))?.len(),
            SIZE,
            "{format:?} should extract the full member"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 1, "{format:?}");
    }
    Ok(())
}

#[test]
fn truncated_headers_have_clear_errors() -> Result<(), Box<dyn Error>> {
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("file.txt");
        fs::write(&source, b"harmless contents")?;
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, &[source], format, None)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&archive)?
            .set_len(12)?;
        let destination = tempfile::tempdir()?;
        let result = decode_fixture(
            &archive,
            destination.path(),
            format,
            None,
            &Arc::new(AtomicUsize::new(0)),
        );
        let Err(error) = result else {
            panic!("accepted truncated {format:?}")
        };
        assert_eq!(error.to_string(), super::INVALID_ARCHIVE, "{format:?}");
        assert!(destination.path().read_dir()?.next().is_none());
    }
    Ok(())
}

#[test]
fn gzip_trailer_mismatch_fails_after_the_last_member() -> Result<(), Box<dyn Error>> {
    let large = vec![b'x'; 50_000];
    for (offset_from_end, label) in [(8, "crc32"), (4, "isize")] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("content.tar.gz");
        write_tar_entries(
            &archive,
            &[
                (tar::EntryType::Regular, "a.txt", &large),
                (tar::EntryType::Regular, "b.txt", b"b"),
            ],
            true,
        )?;
        corrupt_gzip_trailer(&archive, offset_from_end)?;
        let destination = tempfile::tempdir()?;
        fs::write(destination.path().join("b.txt"), b"existing")?;
        let progress = Arc::new(AtomicUsize::new(0));

        let result = extract_tar(
            &archive,
            destination.path(),
            "content.tar.gz",
            true,
            &progress,
            &never_cancelled(),
        );

        let kept = format!(
            "{} Extracted entries remain in `content`.",
            super::INVALID_ARCHIVE
        );
        assert!(
            matches!(&result, Err(ArchiveError::Failed(message)) if *message == kept),
            "{label}: {result:?}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 2, "{label}");
        assert_eq!(
            fs::metadata(destination.path().join("content/a.txt"))?.len(),
            50_000,
            "{label}"
        );
        assert_eq!(
            fs::read(destination.path().join("content/b.txt"))?,
            b"b",
            "{label}"
        );
        assert_eq!(
            fs::read(destination.path().join("b.txt"))?,
            b"existing",
            "{label}"
        );
        assert_eq!(destination.path().read_dir()?.count(), 2, "{label}");
    }
    Ok(())
}

#[test]
fn truncated_gzip_trailer_is_damaged() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("content.tar.gz");
    write_tar_entries(&archive, &[(tar::EntryType::Regular, "a.txt", b"a")], true)?;
    let length = fs::metadata(&archive)?.len();
    fs::OpenOptions::new()
        .write(true)
        .open(&archive)?
        .set_len(length - 3)?;
    let members = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&archive)?))
        .entries()?
        .count();
    assert_eq!(members, 1, "the TAR stream must stay readable");
    let destination = tempfile::tempdir()?;

    let result = extract_tar(
        &archive,
        destination.path(),
        "content.tar.gz",
        true,
        &Arc::new(AtomicUsize::new(0)),
        &never_cancelled(),
    );

    let kept = format!(
        "{} Extracted entries remain in `content`.",
        super::INVALID_ARCHIVE
    );
    assert!(
        matches!(&result, Err(ArchiveError::Failed(message)) if *message == kept),
        "{result:?}"
    );
    assert_eq!(fs::read(destination.path().join("content/a.txt"))?, b"a");
    assert_eq!(destination.path().read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn gzip_trailer_check_is_cancellable() -> Result<(), Box<dyn Error>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&[b'x'; 4096])?;
    let valid = encoder.finish()?;
    let mut corrupt = valid.clone();
    let crc = corrupt.len() - 8;
    corrupt[crc] ^= 0xff;
    for (label, stream, cancelled, expected) in [
        ("valid", &valid, never_cancelled(), Ok(())),
        (
            "cancelled",
            &valid,
            always_cancelled(),
            Err(ArchiveError::Cancelled),
        ),
        (
            "corrupt",
            &corrupt,
            never_cancelled(),
            Err(ArchiveError::Failed(super::INVALID_ARCHIVE.to_owned())),
        ),
    ] {
        let decoder = flate2::read::GzDecoder::new(Cursor::new(stream));
        assert_eq!(
            super::verify_gzip_trailer(decoder, &cancelled),
            expected,
            "{label}"
        );
    }
    Ok(())
}

#[test]
fn corrupt_members_are_removed_without_losing_completed_or_existing_files()
-> Result<(), Box<dyn Error>> {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("archive.zip");
        let entries = [
            ("done.txt", b"done".as_slice()),
            ("broken.txt", b"payload".as_slice()),
        ];
        if format == ArchiveFormat::Zip {
            super::super::fixtures::write_zip_stored(&archive, &entries)?;
        } else {
            let mut writer = sevenz_rust2::ArchiveWriter::create(&archive)?;
            writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
                sevenz_rust2::EncoderMethod::COPY,
            )]);
            for (name, contents) in entries {
                writer.push_archive_entry(
                    sevenz_rust2::ArchiveEntry::new_file(name),
                    Some(Cursor::new(contents)),
                )?;
            }
            writer.finish()?;
        }
        let mut bytes = fs::read(&archive)?;
        let offset = bytes
            .windows(7)
            .position(|bytes| bytes == b"payload")
            .expect("stored payload");
        bytes[offset] ^= 1;
        fs::write(&archive, &bytes)?;
        let destination = tempfile::tempdir()?;
        fs::write(destination.path().join("broken.txt"), b"original")?;
        let progress = Arc::new(AtomicUsize::new(0));
        let result = decode_fixture(&archive, destination.path(), format, None, &progress);
        let kept = format!(
            "{} Extracted entries remain in `archive`.",
            super::INVALID_ARCHIVE
        );
        assert!(
            matches!(&result, Err(ArchiveError::Failed(message)) if *message == kept),
            "{format:?}: {result:?}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 1);
        assert_eq!(
            fs::read(destination.path().join("archive/done.txt"))?,
            b"done"
        );
        assert!(!destination.path().join("archive/broken.txt").exists());
        assert_eq!(
            fs::read(destination.path().join("broken.txt"))?,
            b"original"
        );
        assert_eq!(destination.path().read_dir()?.count(), 2);
        assert_eq!(fs::read(&archive)?, bytes);
    }
    Ok(())
}

#[test]
fn error_translation_preserves_passwords_unsupported_formats_and_io_failures() {
    use sevenz_rust2::Error as SevenZError;
    use zip::result::ZipError;
    for error in [
        ZipError::InvalidPassword,
        ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED),
        ZipError::UnsupportedArchive("unsupported encryption"),
        ZipError::CompressionMethodNotSupported(99),
        ZipError::FileNotFound,
    ] {
        let expected = error.to_string();
        assert_eq!(super::zip_error(error).to_string(), expected);
    }
    assert_eq!(
        super::sevenz_decode_error(SevenZError::PasswordRequired).to_string(),
        "A password is required to extract this archive."
    );
    assert_eq!(
        super::sevenz_decode_error(SevenZError::MaybeBadPassword(
            io::ErrorKind::InvalidData.into()
        ))
        .to_string(),
        "The password may be incorrect."
    );
    for error in [
        SevenZError::UnsupportedVersion { major: 9, minor: 0 },
        SevenZError::UnsupportedCompressionMethod("unknown".into()),
        SevenZError::Unsupported("unsupported encryption".into()),
        SevenZError::FileNotFound,
        SevenZError::Other("unknown decoder failure".into()),
    ] {
        let expected = error.to_string();
        assert_eq!(super::sevenz_decode_error(error).to_string(), expected);
    }
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
        io::ErrorKind::StorageFull,
        io::ErrorKind::Unsupported,
        io::ErrorKind::Interrupted,
        io::ErrorKind::InvalidInput,
        io::ErrorKind::Other,
    ] {
        let error = io::Error::new(kind, "injected I/O failure");
        let translated = super::archive_read_error(error, false);
        assert_eq!(translated.kind(), kind);
        assert_eq!(translated.to_string(), "injected I/O failure");
    }
}

fn write_zipcrypto(
    path: &Path,
    password: &[u8],
    name: &str,
    contents: &[u8],
    compression: zip::CompressionMethod,
) -> Result<(), Box<dyn Error>> {
    use zip::unstable::write::FileOptionsExt;
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(compression)
        .with_deprecated_encryption(password)?;
    writer.start_file(name, options)?;
    writer.write_all(contents)?;
    writer.finish()?;
    Ok(())
}

fn zipcrypto_crc_collision(archive_path: &Path) -> Result<String, Box<dyn Error>> {
    for candidate in 0..4096u32 {
        let password = candidate.to_string();
        let file = fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let options = zip::read::ZipReadOptions::new().password(Some(password.as_bytes()));
        let mut entry = match archive.by_index_with_options(0, options) {
            Err(zip::result::ZipError::InvalidPassword) => continue,
            Err(error) => return Err(error.into()),
            Ok(entry) => entry,
        };
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() {
            return Ok(password);
        }
    }
    Err("no ZipCrypto CRC collision in 0..4096".into())
}

fn zipcrypto_fixture(root: &Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let archive = root.join("password.zip");
    write_zipcrypto(
        &archive,
        b"zipsecret",
        "some.txt",
        b"hello from zipcrypto",
        zip::CompressionMethod::Stored,
    )?;
    Ok(archive)
}

fn zipcrypto_collision_is_retryable(
    compression: zip::CompressionMethod,
    contents: &[u8],
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("password.zip");
    write_zipcrypto(&archive, b"zipsecret", "some.txt", contents, compression)?;
    let collision = zipcrypto_crc_collision(&archive)?;

    let destination = tempfile::tempdir()?;
    let Err(error) = decode_fixture(
        &archive,
        destination.path(),
        ArchiveFormat::Zip,
        Some(&collision),
        &Arc::new(AtomicUsize::new(0)),
    ) else {
        panic!("ZipCrypto {compression:?} collision {collision} extracted");
    };
    assert_eq!(error.to_string(), super::MAYBE_BAD_PASSWORD);
    assert!(destination.path().read_dir()?.next().is_none());

    let correct = tempfile::tempdir()?;
    completed_extract(decode_fixture(
        &archive,
        correct.path(),
        ArchiveFormat::Zip,
        Some("zipsecret"),
        &Arc::new(AtomicUsize::new(0)),
    )?)?;
    assert_eq!(fs::read(correct.path().join("some.txt"))?, contents);
    Ok(())
}

#[test]
fn zipcrypto_header_collision_is_retryable() -> Result<(), Box<dyn Error>> {
    zipcrypto_collision_is_retryable(zip::CompressionMethod::Stored, b"hello from zipcrypto")
}

#[test]
fn zipcrypto_deflated_header_collision_is_retryable() -> Result<(), Box<dyn Error>> {
    zipcrypto_collision_is_retryable(
        zip::CompressionMethod::Deflated,
        &b"hello from zipcrypto\n".repeat(64),
    )
}

fn zipcrypto_rejected_password(path: &Path) -> Result<String, Box<dyn Error>> {
    // ZipCrypto's one-byte header check can accept an arbitrary wrong password.
    for candidate in 0..4096 {
        let password = format!("wrong-{candidate}");
        let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
        let options = zip::read::ZipReadOptions::new().password(Some(password.as_bytes()));
        match archive.by_index_with_options(0, options) {
            Err(zip::result::ZipError::InvalidPassword) => return Ok(password),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
    }
    Err("no header-rejected ZipCrypto password found".into())
}

#[test]
fn zipcrypto_wrong_password_stays_invalid_password() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = zipcrypto_fixture(root.path())?;
    let password = zipcrypto_rejected_password(&archive)?;
    let destination = tempfile::tempdir()?;
    let Err(error) = decode_fixture(
        &archive,
        destination.path(),
        ArchiveFormat::Zip,
        Some(&password),
        &Arc::new(AtomicUsize::new(0)),
    ) else {
        panic!("ZipCrypto accepted a non-colliding wrong password");
    };
    assert_eq!(error.to_string(), "provided password is incorrect");
    assert!(destination.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn zipcrypto_without_password_still_requires_one() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = zipcrypto_fixture(root.path())?;
    let destination = tempfile::tempdir()?;
    let Err(error) = decode_fixture(
        &archive,
        destination.path(),
        ArchiveFormat::Zip,
        None,
        &Arc::new(AtomicUsize::new(0)),
    ) else {
        panic!("ZipCrypto extracted without a password");
    };
    assert!(error.to_string().contains("Password required"), "{error}");
    assert!(destination.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn aes_zip_wrong_password_stays_invalid_password() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("file.txt");
    fs::write(&source, b"contents")?;
    let archive = root.path().join("archive.zip");
    write_compression_fixture(
        &archive,
        &[source],
        ArchiveFormat::Zip,
        Some("test-password"),
    )?;

    let destination = tempfile::tempdir()?;
    let Err(error) = decode_fixture(
        &archive,
        destination.path(),
        ArchiveFormat::Zip,
        Some("wrong"),
        &Arc::new(AtomicUsize::new(0)),
    ) else {
        panic!("AES zip accepted a wrong password");
    };
    assert_eq!(error.to_string(), "provided password is incorrect");
    assert!(destination.path().read_dir()?.next().is_none());

    let correct = tempfile::tempdir()?;
    completed_extract(decode_fixture(
        &archive,
        correct.path(),
        ArchiveFormat::Zip,
        Some("test-password"),
        &Arc::new(AtomicUsize::new(0)),
    )?)?;
    assert_eq!(fs::read(correct.path().join("file.txt"))?, b"contents");
    Ok(())
}

#[test]
fn unencrypted_zip_checksum_failure_stays_damaged() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive.zip");
    super::super::fixtures::write_zip_stored(&archive, &[("file.txt", b"checksum-payload")])?;
    let mut bytes = fs::read(&archive)?;
    let offset = bytes
        .windows(b"checksum-payload".len())
        .position(|bytes| bytes == b"checksum-payload")
        .expect("stored payload");
    bytes[offset] ^= 1;
    fs::write(&archive, &bytes)?;

    let destination = tempfile::tempdir()?;
    let Err(error) = decode_fixture(
        &archive,
        destination.path(),
        ArchiveFormat::Zip,
        None,
        &Arc::new(AtomicUsize::new(0)),
    ) else {
        panic!("accepted a checksum-damaged zip");
    };
    assert_eq!(error.to_string(), super::INVALID_ARCHIVE);
    assert!(destination.path().read_dir()?.next().is_none());
    Ok(())
}

#[test]
fn wrong_password_for_content_encrypted_7z_is_retryable() -> Result<(), Box<dyn Error>> {
    // Generated with `7z a -psecret -mhe=off` using 7-Zip 26.02.
    let archive = include_bytes!("fixtures/content-encrypted.7z");
    let wrong_destination = tempfile::tempdir()?;
    let Err(error) = extract_7z_from_reader(
        Cursor::new(archive),
        wrong_destination.path(),
        "archive",
        "wrong".into(),
        &Arc::new(AtomicUsize::new(0)),
        &never_cancelled(),
    ) else {
        panic!("a wrong password must fail extraction");
    };
    assert_eq!(error.to_string(), super::MAYBE_BAD_PASSWORD);
    assert!(wrong_destination.path().read_dir()?.next().is_none());

    let correct_destination = tempfile::tempdir()?;
    completed_extract(extract_7z_from_reader(
        Cursor::new(archive),
        correct_destination.path(),
        "archive",
        "secret".into(),
        &Arc::new(AtomicUsize::new(0)),
        &never_cancelled(),
    )?)?;
    assert_eq!(
        fs::read(correct_destination.path().join("document.txt"))?,
        b"private contents"
    );
    Ok(())
}

#[test]
fn checksum_failure_without_a_password_remains_damaged() {
    let error = io::Error::other(sevenz_rust2::Error::ChecksumVerificationFailed);
    assert_eq!(
        super::archive_read_error(error, false).to_string(),
        super::INVALID_ARCHIVE
    );
}

#[test]
fn corrupt_deflate_stream_without_a_password_stays_damaged() {
    let error = io::Error::new(io::ErrorKind::InvalidInput, "corrupt deflate stream");
    assert_eq!(
        super::archive_read_error(error, false).to_string(),
        super::INVALID_ARCHIVE
    );
}

#[test]
fn corrupt_deflate_stream_after_a_password_is_retryable() {
    let error = io::Error::new(io::ErrorKind::InvalidInput, "corrupt deflate stream");
    assert_eq!(
        super::archive_read_error(error, true).to_string(),
        super::MAYBE_BAD_PASSWORD
    );
}

#[test]
fn tar_extraction_skips_pax_global_headers() -> Result<(), Box<dyn Error>> {
    for gzip in [false, true] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir_all(&destination)?;
        let archive = root.path().join("project.tar");
        write_tar_entries(
            &archive,
            &[
                (
                    tar::EntryType::XGlobalHeader,
                    "pax_global_header",
                    b"52 comment=0123456789abcdef0123456789abcdef01234567\n".as_slice(),
                ),
                (tar::EntryType::Directory, "project/", b"".as_slice()),
                (tar::EntryType::Regular, "project/README", b"hello"),
            ],
            gzip,
        )?;
        let progress = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            completed_extract(extract_tar(
                &archive,
                &destination,
                "archive",
                gzip,
                &progress,
                &never_cancelled(),
            )?)?,
            Some("project".to_owned()),
        );
        assert_eq!(progress.load(Ordering::Relaxed), 2);
        assert!(!destination.join("pax_global_header").exists());
        assert_eq!(fs::read(destination.join("project/README"))?, b"hello");
        assert_eq!(fs::read_dir(&destination)?.count(), 1);
    }
    Ok(())
}

const STORED_TIME: u64 = 1_000_000_000;

fn names_in(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, io::Error>>()?;
    names.sort();
    Ok(names)
}

fn kind_of(path: &Path) -> String {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => "a symlink".to_owned(),
        Ok(metadata) if metadata.is_dir() => "a directory".to_owned(),
        Ok(metadata) => format!("a regular file of {} bytes", metadata.len()),
        Err(error) => format!("missing ({error})"),
    }
}

fn assert_symlink(path: &Path, target: &str, context: &str) -> Result<(), Box<dyn Error>> {
    assert!(
        fs::symlink_metadata(path)?.file_type().is_symlink(),
        "{context}: `{}` extracted as {}, not as a symlink to `{target}`",
        path.display(),
        kind_of(path)
    );
    assert_eq!(fs::read_link(path)?, Path::new(target), "{context}");
    Ok(())
}

fn assert_mode_and_time(
    path: &Path,
    stored_mode: u32,
    modified: i64,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let metadata = fs::metadata(path)?;
    assert_eq!(
        format!("{:o}", metadata.permissions().mode() & 0o7777),
        format!("{:o}", expected_mode(stored_mode)),
        "{context}: mode of `{}` (stored {stored_mode:o})",
        path.display()
    );
    assert_eq!(
        metadata.mtime(),
        modified,
        "{context}: mtime of `{}`",
        path.display()
    );
    Ok(())
}

#[test]
fn links_extract_as_links_in_every_format() -> Result<(), Box<dyn Error>> {
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        let context = format!("{format:?}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join(format!("links.{}", format.extension()));
        let tar = matches!(format, ArchiveFormat::Tar | ArchiveFormat::TarGz);
        // The ZIP writer refuses duplicate member names.
        let duplicate = format != ArchiveFormat::Zip;
        let mut members = vec![
            FixtureMember::File {
                name: "data.txt",
                contents: b"payload",
                mode: None,
                modified: None,
            },
            FixtureMember::Symlink {
                name: "lnk",
                target: b"data.txt",
            },
            FixtureMember::Symlink {
                name: "abs",
                target: b"/etc/hostname",
            },
            FixtureMember::Symlink {
                name: "nested/up",
                target: b"../data.txt",
            },
        ];
        if duplicate {
            members.push(FixtureMember::Symlink {
                name: "lnk",
                target: b"abs",
            });
        }
        if tar {
            members.push(FixtureMember::HardLink {
                name: "hard",
                target: "data.txt",
            });
        }
        write_members(&archive, format, &members)?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let progress = Arc::new(AtomicUsize::new(0));

        let first_name = completed_extract(decode_fixture(
            &archive,
            &destination,
            format,
            None,
            &progress,
        )?)?;

        assert_eq!(first_name.as_deref(), Some("links"), "{context}");
        assert_eq!(progress.load(Ordering::Relaxed), members.len(), "{context}");
        let output = destination.join("links");
        assert_symlink(&output.join("lnk"), "data.txt", &context)?;
        assert_symlink(&output.join("abs"), "/etc/hostname", &context)?;
        assert_symlink(&output.join("nested/up"), "../data.txt", &context)?;
        assert_eq!(fs::read(output.join("lnk"))?, b"payload", "{context}");
        assert_eq!(fs::read(output.join("nested/up"))?, b"payload", "{context}");
        if duplicate {
            assert_symlink(&output.join("lnk (2)"), "abs", &context)?;
        }
        if tar {
            assert_eq!(
                fs::metadata(output.join("hard"))?.ino(),
                fs::metadata(output.join("data.txt"))?.ino(),
                "{context}: `hard` extracted as {}, not as a hard link to `data.txt`",
                kind_of(&output.join("hard"))
            );
            assert_eq!(fs::read(output.join("hard"))?, b"payload", "{context}");
        }
        assert_eq!(names_in(&destination)?, ["links"], "{context}");
    }
    Ok(())
}

#[test]
fn unsupported_tar_members_are_refused_with_their_name() -> Result<(), Box<dyn Error>> {
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        for (name, entry_type) in [
            ("pipe", Some(tar::EntryType::Fifo)),
            ("char-device", Some(tar::EntryType::Char)),
            ("block-device", Some(tar::EntryType::Block)),
            ("dangling", None),
        ] {
            let context = format!("{format:?} {name}");
            let root = tempfile::tempdir()?;
            let archive = root.path().join(format!("special.{}", format.extension()));
            let gzip = format == ArchiveFormat::TarGz;
            match entry_type {
                Some(entry_type) => write_tar_entries(
                    &archive,
                    &[
                        (tar::EntryType::Regular, "ok.txt", b"ok"),
                        (entry_type, name, b""),
                    ],
                    gzip,
                )?,
                None => write_members(
                    &archive,
                    format,
                    &[
                        FixtureMember::File {
                            name: "ok.txt",
                            contents: b"ok",
                            mode: None,
                            modified: None,
                        },
                        FixtureMember::HardLink {
                            name,
                            target: "missing.txt",
                        },
                    ],
                )?,
            }
            let destination = root.path().join("destination");
            fs::create_dir(&destination)?;
            let progress = Arc::new(AtomicUsize::new(0));

            let result = decode_fixture(&archive, &destination, format, None, &progress);

            let output = destination.join("special");
            let message = match result {
                Err(ArchiveError::Failed(message)) => message,
                other => panic!(
                    "{context}: expected a failure naming `{name}`, got {other:?}; \
                     `special/{name}` is {}",
                    kind_of(&output.join(name))
                ),
            };
            assert!(message.contains(name), "{context}: {message}");
            assert!(
                message.contains("remain in `special`"),
                "{context}: {message}"
            );
            assert_eq!(fs::read(output.join("ok.txt"))?, b"ok", "{context}");
            assert!(
                fs::symlink_metadata(output.join(name)).is_err(),
                "{context}"
            );
            assert!(!destination.join("ok.txt").exists(), "{context}");
            assert_eq!(progress.load(Ordering::Relaxed), 1, "{context}");
        }
    }
    Ok(())
}

#[test]
fn a_member_cannot_be_written_through_an_extracted_symlink() -> Result<(), Box<dyn Error>> {
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
    ] {
        let context = format!("{format:?}");
        let root = tempfile::tempdir()?;
        let external = root.path().join("external");
        fs::create_dir(&external)?;
        let target = external.to_str().ok_or("non-UTF-8 temporary path")?;
        let archive = root.path().join(format!("escape.{}", format.extension()));
        write_members(
            &archive,
            format,
            &[
                FixtureMember::Symlink {
                    name: "lnk",
                    target: target.as_bytes(),
                },
                FixtureMember::File {
                    name: "lnk/escape.txt",
                    contents: b"escaped",
                    mode: None,
                    modified: None,
                },
            ],
        )?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;

        let result = decode_fixture(
            &archive,
            &destination,
            format,
            None,
            &Arc::new(AtomicUsize::new(0)),
        );

        assert!(
            matches!(result, Err(ArchiveError::Failed(_))),
            "{context}: {result:?}; destination holds {:?}",
            names_in(&destination)?
        );
        assert!(!external.join("escape.txt").exists(), "{context}");
        assert_symlink(&destination.join("escape/lnk"), target, &context)?;
    }
    Ok(())
}

#[test]
fn member_modes_and_times_are_restored_in_every_format() -> Result<(), Box<dyn Error>> {
    let stored = Some(STORED_TIME);
    let members = [
        FixtureMember::File {
            name: "bin/run.sh",
            contents: b"#!/bin/sh\necho ok\n",
            mode: Some(0o755),
            modified: stored,
        },
        FixtureMember::File {
            name: "data.txt",
            contents: b"private",
            mode: Some(0o600),
            modified: stored,
        },
        FixtureMember::File {
            name: "suid",
            contents: b"setuid",
            mode: Some(0o4755),
            modified: stored,
        },
        FixtureMember::File {
            name: "readonly.txt",
            contents: b"read only",
            mode: Some(0o444),
            modified: stored,
        },
        FixtureMember::Directory {
            name: "ro",
            mode: Some(0o555),
            modified: stored,
        },
        FixtureMember::File {
            name: "ro/child.txt",
            contents: b"child",
            mode: Some(0o644),
            modified: stored,
        },
    ];
    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
    ] {
        let context = format!("{format:?}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join(format!("modes.{}", format.extension()));
        write_members(&archive, format, &members)?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let progress = Arc::new(AtomicUsize::new(0));

        let first_name = completed_extract(decode_fixture(
            &archive,
            &destination,
            format,
            None,
            &progress,
        )?)?;

        assert_eq!(first_name.as_deref(), Some("modes"), "{context}");
        assert_eq!(progress.load(Ordering::Relaxed), members.len(), "{context}");
        let output = destination.join("modes");
        let time = i64::try_from(STORED_TIME)?;
        for (name, mode) in [
            ("bin/run.sh", 0o755),
            ("data.txt", 0o600),
            ("suid", 0o4755),
            ("readonly.txt", 0o444),
            ("ro/child.txt", 0o644),
            ("ro", 0o555),
        ] {
            assert_mode_and_time(&output.join(name), mode, time, &context)?;
        }
        fs::set_permissions(output.join("ro"), fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[test]
fn members_without_a_usable_mode_keep_the_default_permissions() -> Result<(), Box<dyn Error>> {
    // ZIP external attributes `0x20` carry only the DOS archive bit, so a
    // Unix creator reports mode 0. The 7z junk mode has no regular-file type.
    for (format, mode, zip_attributes) in [
        (ArchiveFormat::Zip, None, Some(0)),
        (ArchiveFormat::Zip, None, Some(0x20)),
        (ArchiveFormat::SevenZ, None, None),
        (ArchiveFormat::SevenZ, Some(0o070_644), None),
    ] {
        let context = format!("{format:?} {mode:?} {zip_attributes:?}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join(format!("plain.{}", format.extension()));
        write_members(
            &archive,
            format,
            &[FixtureMember::File {
                name: "plain.txt",
                contents: b"plain",
                mode,
                modified: None,
            }],
        )?;
        if let Some(attributes) = zip_attributes {
            patch_zip_external_attributes(&archive, attributes)?;
        }
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        completed_extract(decode_fixture(
            &archive,
            &destination,
            format,
            None,
            &Arc::new(AtomicUsize::new(0)),
        )?)?;

        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let extracted = destination.join("plain.txt");
        if format == ArchiveFormat::Zip {
            // The DOS default (1980-01-01 local) is a stored time.
            let dos_default = glib::DateTime::from_local(1980, 1, 1, 0, 0, 0.0)?.to_unix();
            assert_mode_and_time(&extracted, 0o666, dos_default, &context)?;
        } else {
            let modified = u64::try_from(fs::metadata(&extracted)?.mtime())?;
            assert!((before..=after).contains(&modified), "{context}");
            assert_eq!(
                fs::metadata(&extracted)?.permissions().mode() & 0o7777,
                expected_mode(0o666),
                "{context}"
            );
        }
    }
    Ok(())
}

#[test]
fn zip_prefers_the_extended_timestamp_over_the_dos_time() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("times.zip");
    let dos = zip::DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0)?;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(dos);
    let mut extended = options.into_full_options();
    extended.add_extra_data(0x5455, zip_extended_timestamp(STORED_TIME)?, false)?;
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
    writer.start_file("extended.txt", extended)?;
    writer.write_all(b"extended")?;
    writer.start_file("dos.txt", options)?;
    writer.write_all(b"dos")?;
    // A signed UT time of -1 is before 1970, so the DOS time applies.
    let mut negative = options.into_full_options();
    negative.add_extra_data(0x5455, [1, 0xff, 0xff, 0xff, 0xff], false)?;
    writer.start_file("negative.txt", negative)?;
    writer.write_all(b"negative")?;
    writer.finish()?;
    let mut reader = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    assert!(
        reader
            .by_name("extended.txt")?
            .extra_data_fields()
            .any(|field| matches!(
                field,
                zip::ExtraField::ExtendedTimestamp(stamp)
                    if stamp.mod_time().map(u64::from) == Some(STORED_TIME)
            )),
        "fixture lacks the UT field"
    );
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;

    let first_name = completed_extract(decode_fixture(
        &archive,
        &destination,
        ArchiveFormat::Zip,
        None,
        &Arc::new(AtomicUsize::new(0)),
    )?)?;

    assert_eq!(first_name.as_deref(), Some("times"));
    assert_eq!(
        fs::metadata(destination.join("times/extended.txt"))?.mtime(),
        i64::try_from(STORED_TIME)?
    );
    let dos_local = glib::DateTime::from_local(2000, 1, 1, 0, 0, 0.0)?.to_unix();
    for name in ["dos.txt", "negative.txt"] {
        let mtime = fs::metadata(destination.join("times").join(name))?.mtime();
        assert_eq!(mtime, dos_local, "{name}");
    }
    Ok(())
}

#[test]
fn every_gzip_member_of_a_tar_gz_is_read_and_verified() -> Result<(), Box<dyn Error>> {
    for corrupt_second_member in [false, true] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("content.tar.gz");
        write_tar_entries(
            &archive,
            &[
                (tar::EntryType::Regular, "a.txt", b"a"),
                (tar::EntryType::Regular, "b.txt", b"b"),
            ],
            false,
        )?;
        // After `a.txt`'s header and data blocks, so a reader that stops at
        // the first member sees a complete archive.
        split_into_gzip_members(&archive, 1024)?;
        if corrupt_second_member {
            corrupt_gzip_trailer(&archive, 8)?;
        }
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;

        let result = extract_tar(
            &archive,
            &destination,
            "content.tar.gz",
            true,
            &Arc::new(AtomicUsize::new(0)),
            &never_cancelled(),
        );

        if corrupt_second_member {
            let kept = format!(
                "{} Extracted entries remain in `content`.",
                super::INVALID_ARCHIVE
            );
            assert!(
                matches!(&result, Err(ArchiveError::Failed(message)) if *message == kept),
                "{result:?}"
            );
        } else {
            assert_eq!(completed_extract(result?)?.as_deref(), Some("content"));
        }
        assert_eq!(fs::read(destination.join("content/a.txt"))?, b"a");
        assert_eq!(fs::read(destination.join("content/b.txt"))?, b"b");
    }
    Ok(())
}
