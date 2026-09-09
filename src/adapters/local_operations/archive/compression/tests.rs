// SPDX-License-Identifier: MIT

use super::super::fixtures::{
    compression_stage_mode, compression_stages, never_cancelled, write_compression_fixture,
};
use super::{ArchiveError, count_archive_files, process_umask, write_staged_archive};
use crate::{
    services::{ArchiveFormat, TransferConflict},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};
use gtk::glib;
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    fs,
    io::{Read, Write},
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[test]
fn compression_staging_stays_private_while_encoding() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("existing.zip");
    fs::write(&archive, b"original")?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_release = release.clone();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            &never_cancelled(),
            move |mut file| {
                file.write_all(b"replacement")
                    .map_err(|error| error.to_string())?;
                worker_started.store(true, Ordering::Release);
                while !worker_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                Ok(())
            },
        )
        .await
    });
    let context = glib::MainContext::default();
    while !started.load(Ordering::Acquire) {
        context.iteration(false);
        std::thread::yield_now();
    }
    assert_eq!(compression_stage_mode(&destination)?, 0o600);

    release.store(true, Ordering::Release);
    assert_eq!(context.block_on(task)?, Ok(()));
    assert_eq!(fs::read(&archive)?, b"replacement");
    assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_new_archive_staging_stays_private_until_publish() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("created.zip");
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_release = release.clone();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::FailIfExists,
            &never_cancelled(),
            move |mut file| {
                file.write_all(b"created")
                    .map_err(|error| error.to_string())?;
                worker_started.store(true, Ordering::Release);
                while !worker_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                Ok(())
            },
        )
        .await
    });
    let context = glib::MainContext::default();
    while !started.load(Ordering::Acquire) {
        context.iteration(false);
        std::thread::yield_now();
    }
    assert_eq!(compression_stage_mode(&destination)?, 0o600);

    release.store(true, Ordering::Release);
    assert_eq!(context.block_on(task)?, Ok(()));
    assert_eq!(fs::read(&archive)?, b"created");
    assert_eq!(
        fs::metadata(&archive)?.permissions().mode() & 0o777,
        0o666 & !process_umask()
    );
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum CompressedEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

fn read_compressed_entries(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<BTreeMap<PathBuf, CompressedEntry>, Box<dyn Error>> {
    let file = fs::File::open(path)?;
    let mut result = BTreeMap::new();
    match format {
        ArchiveFormat::Zip => {
            let mut archive = zip::ZipArchive::new(file)?;
            for index in 0..archive.len() {
                let options =
                    zip::read::ZipReadOptions::new().password(password.map(str::as_bytes));
                let mut entry = archive.by_index_with_options(index, options)?;
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                let value = if entry.is_dir() {
                    CompressedEntry::Directory
                } else if entry.is_symlink() {
                    CompressedEntry::Symlink(PathBuf::from(OsString::from_vec(bytes)))
                } else {
                    CompressedEntry::File(bytes)
                };
                assert!(result.insert(PathBuf::from(entry.name()), value).is_none());
            }
        }
        ArchiveFormat::Tar | ArchiveFormat::TarGz => {
            let reader: Box<dyn Read> = if format == ArchiveFormat::TarGz {
                Box::new(flate2::read::GzDecoder::new(file))
            } else {
                Box::new(file)
            };
            for entry in tar::Archive::new(reader).entries()? {
                let mut entry = entry?;
                let value = if entry.header().entry_type().is_dir() {
                    CompressedEntry::Directory
                } else if entry.header().entry_type().is_symlink() {
                    CompressedEntry::Symlink(
                        entry
                            .link_name()?
                            .ok_or("Missing link target")?
                            .into_owned(),
                    )
                } else {
                    assert!(entry.header().entry_type().is_file());
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes)?;
                    CompressedEntry::File(bytes)
                };
                assert!(result.insert(entry.path()?.into_owned(), value).is_none());
            }
        }
        ArchiveFormat::SevenZ => {
            let mut archive = sevenz_rust2::ArchiveReader::new(
                file,
                password
                    .map(sevenz_rust2::Password::from)
                    .unwrap_or_default(),
            )?;
            archive.for_each_entries(|entry, reader| {
                let value = if entry.is_directory() {
                    CompressedEntry::Directory
                } else {
                    let mut bytes = Vec::new();
                    reader.read_to_end(&mut bytes)?;
                    CompressedEntry::File(bytes)
                };
                assert!(result.insert(PathBuf::from(entry.name()), value).is_none());
                Ok(true)
            })?;
        }
    }
    Ok(result)
}

#[test]
fn compression_preserves_links_in_zip_and_tar() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir(source.join("empty"))?;
    fs::write(source.join("file.txt"), b"file contents")?;
    fs::write(source.join("nested/child.txt"), b"child contents")?;
    let mut expected = BTreeMap::from([
        (PathBuf::from("source"), CompressedEntry::Directory),
        (PathBuf::from("source/nested"), CompressedEntry::Directory),
        (PathBuf::from("source/empty"), CompressedEntry::Directory),
        (
            PathBuf::from("source/file.txt"),
            CompressedEntry::File(b"file contents".to_vec()),
        ),
        (
            PathBuf::from("source/nested/child.txt"),
            CompressedEntry::File(b"child contents".to_vec()),
        ),
    ]);
    for (name, target) in [
        ("file-link", "file.txt"),
        ("directory-link", "nested"),
        ("broken-link", "missing.txt"),
        ("current-directory-link", "."),
    ] {
        std::os::unix::fs::symlink(target, source.join(name))?;
        expected.insert(
            PathBuf::from("source").join(name),
            CompressedEntry::Symlink(PathBuf::from(target)),
        );
    }
    let selected_link = root.path().join("selected-link");
    std::os::unix::fs::symlink("source/nested", &selected_link)?;
    expected.insert(
        PathBuf::from("selected-link"),
        CompressedEntry::Symlink(PathBuf::from("source/nested")),
    );
    let entries = [source, selected_link];
    assert_eq!(count_archive_files(&entries, &never_cancelled())?, 7);
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
    ] {
        let archive = root.path().join("archive");
        assert_eq!(
            write_compression_fixture(&archive, &entries, format, password)?,
            7
        );
        assert_eq!(
            read_compressed_entries(&archive, format, password)?,
            expected,
            "{format:?}"
        );
    }
    Ok(())
}

#[test]
fn seven_z_compression_preserves_files_and_empty_directories() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("empty"))?;
    fs::write(source.join("one.txt"), b"one")?;
    fs::write(source.join("two.png"), b"two")?;
    let expected = BTreeMap::from([
        (PathBuf::from("source"), CompressedEntry::Directory),
        (PathBuf::from("source/empty"), CompressedEntry::Directory),
        (
            PathBuf::from("source/one.txt"),
            CompressedEntry::File(b"one".to_vec()),
        ),
        (
            PathBuf::from("source/two.png"),
            CompressedEntry::File(b"two".to_vec()),
        ),
    ]);
    for password in [None, Some("test-password")] {
        let archive = root.path().join("archive");
        assert_eq!(
            write_compression_fixture(
                &archive,
                std::slice::from_ref(&source),
                ArchiveFormat::SevenZ,
                password
            )?,
            2
        );
        assert_eq!(
            read_compressed_entries(&archive, ArchiveFormat::SevenZ, password)?,
            expected,
        );
    }
    Ok(())
}

#[test]
fn compression_handles_non_utf8_link_targets_without_loss() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let link = root.path().join("link");
    let target = PathBuf::from(OsString::from_vec(b"target-\xff".to_vec()));
    std::os::unix::fs::symlink(&target, &link)?;
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, std::slice::from_ref(&link), format, None)?;
        assert_eq!(
            read_compressed_entries(&archive, format, None)?,
            BTreeMap::from([(
                PathBuf::from("link"),
                CompressedEntry::Symlink(target.clone())
            )])
        );
    }
    let error = write_compression_fixture(
        &root.path().join("archive.zip"),
        &[link],
        ArchiveFormat::Zip,
        None,
    )
    .expect_err("ZIP must reject a link target it cannot encode");
    assert!(error.contains("non-UTF-8 link target"));
    Ok(())
}

#[test]
fn cancelling_staged_compression_unlinks_the_partial_output() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("existing.zip");
    fs::write(&archive, b"original")?;
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_release = release.clone();
    let worker_finished = finished.clone();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            &never_cancelled(),
            move |mut file| {
                file.write_all(b"partial")
                    .map_err(|error| error.to_string())?;
                worker_started.store(true, Ordering::Release);
                while !worker_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                worker_finished.store(true, Ordering::Release);
                Ok(())
            },
        )
        .await
    });
    let context = glib::MainContext::default();
    while !started.load(Ordering::Acquire) {
        context.iteration(false);
        std::thread::yield_now();
    }
    assert_eq!(compression_stages(&destination)?.len(), 1);

    task.abort();
    drop(task);
    while context.pending() {
        context.iteration(false);
    }
    let stage_was_removed = compression_stages(&destination)?.is_empty();
    let destination_was_preserved = fs::read(&archive)? == b"original";
    release.store(true, Ordering::Release);
    while !finished.load(Ordering::Acquire) {
        std::thread::yield_now();
    }

    assert!(stage_was_removed);
    assert!(destination_was_preserved);
    Ok(())
}

#[test]
fn write_staged_archive_does_not_publish_when_cancelled_after_write() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("existing.zip");
    fs::write(&archive, b"original")?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let persist_cancelled = cancelled.clone();
    let worker_cancelled = cancelled.clone();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            &persist_cancelled,
            move |mut file| {
                file.write_all(b"replacement")
                    .map_err(|error| error.to_string())?;
                worker_cancelled.store(true, Ordering::Release);
                Ok(())
            },
        )
        .await
    });
    let result = glib::MainContext::default().block_on(task)?;
    assert!(matches!(result, Err(ArchiveError::Cancelled)));
    assert_eq!(fs::read(&archive)?, b"original");
    assert!(compression_stages(&destination)?.is_empty());
    Ok(())
}

#[test]
fn compression_accepts_a_symlink_in_the_parent_path() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    fs::create_dir_all(actual.join("tree"))?;
    fs::write(actual.join("tree/file.txt"), b"contents")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    let entries = [alias.join("tree")];
    let expected = BTreeMap::from([
        (PathBuf::from("tree"), CompressedEntry::Directory),
        (
            PathBuf::from("tree/file.txt"),
            CompressedEntry::File(b"contents".to_vec()),
        ),
    ]);

    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::SevenZ,
    ] {
        let archive = root.path().join("archive");
        assert_eq!(count_archive_files(&entries, &never_cancelled())?, 1);
        assert_eq!(
            write_compression_fixture(&archive, &entries, format, None)?,
            1
        );
        assert_eq!(
            read_compressed_entries(&archive, format, None)?,
            expected,
            "{format:?}"
        );
    }
    Ok(())
}

#[test]
fn zip_and_seven_z_refuse_non_utf8_names_instead_of_mangling_them() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root
        .path()
        .join(OsString::from_vec(b"name-\xff.txt".to_vec()));
    fs::write(&source, b"contents")?;
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let archive = root.path().join("archive.out");
        let error =
            write_compression_fixture(&archive, std::slice::from_ref(&source), format, None)
                .expect_err("a non-UTF-8 name cannot be stored losslessly");
        assert!(error.contains("non-UTF-8 name"), "{format:?}: {error}");
    }
    let archive = root.path().join("archive.tar");
    write_compression_fixture(
        &archive,
        std::slice::from_ref(&source),
        ArchiveFormat::Tar,
        None,
    )?;
    Ok(())
}
