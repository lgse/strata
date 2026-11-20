// SPDX-License-Identifier: MIT

mod cancellation;
mod damage;
mod errors;
mod layout;
mod metadata;
mod passwords;

use super::super::fixtures::{
    FixtureMember, always_cancelled, completed_extract, corrupt_gzip_trailer, expected_mode,
    extract_zip, never_cancelled, patch_zip_external_attributes, patch_zip_uncompressed_size,
    split_into_gzip_members, write_7z_entries, write_7z_stored, write_compression_fixture,
    write_members, write_tar_entries, write_zip, write_zip_stored, zip_extended_timestamp,
};
use super::{
    ArchiveError, ArchiveOutcome, INVALID_ARCHIVE, extract_7z_from_reader, extract_tar,
    extract_zip_from_archive,
};
use crate::{
    model::Location,
    services::{ArchiveFormat, TransferReport},
};
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

const ALL_FORMATS: [ArchiveFormat; 4] = [
    ArchiveFormat::Zip,
    ArchiveFormat::SevenZ,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
];

const STORED_TIME: u64 = 1_000_000_000;

fn tar_format(gzip: bool) -> ArchiveFormat {
    if gzip {
        ArchiveFormat::TarGz
    } else {
        ArchiveFormat::Tar
    }
}

/// One decoder run with its own member counter and cancellation flag.
struct Decoder<'a> {
    format: ArchiveFormat,
    name: Option<&'a str>,
    password: Option<&'a str>,
    progress: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
}

impl<'a> Decoder<'a> {
    fn new(format: ArchiveFormat) -> Self {
        Self {
            format,
            name: None,
            password: None,
            progress: Arc::new(AtomicUsize::new(0)),
            cancelled: never_cancelled(),
        }
    }

    fn with_password(format: ArchiveFormat, password: Option<&'a str>) -> Self {
        Self {
            password,
            ..Self::new(format)
        }
    }

    /// Names the archive instead of taking the name from its file.
    fn named(self, name: &'a str) -> Self {
        Self {
            name: Some(name),
            ..self
        }
    }

    fn cancelled(self) -> Self {
        Self {
            cancelled: always_cancelled(),
            ..self
        }
    }

    fn progress(&self) -> usize {
        self.progress.load(Ordering::Relaxed)
    }

    fn run(
        &self,
        archive: &Path,
        destination: &Path,
    ) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
        let name = self
            .name
            .or_else(|| archive.file_name().and_then(OsStr::to_str))
            .unwrap_or("archive");
        let mut report = TransferReport::default();
        match self.format {
            ArchiveFormat::Zip => {
                let file = fs::File::open(archive).map_err(super::archive_failed)?;
                let mut archive = zip::ZipArchive::new(file).map_err(super::zip_error)?;
                extract_zip_from_archive(
                    &mut archive,
                    destination,
                    name,
                    self.password,
                    &self.progress,
                    &self.cancelled,
                    &mut report,
                )
            }
            ArchiveFormat::SevenZ => extract_7z_from_reader(
                fs::File::open(archive).map_err(super::archive_failed)?,
                destination,
                name,
                self.password
                    .map(sevenz_rust2::Password::from)
                    .unwrap_or_default(),
                &self.progress,
                &self.cancelled,
                &mut report,
            ),
            ArchiveFormat::Tar | ArchiveFormat::TarGz => extract_tar(
                archive,
                destination,
                name,
                self.format == ArchiveFormat::TarGz,
                &self.progress,
                &self.cancelled,
                &mut report,
            ),
            ArchiveFormat::Rar => unreachable!("no fixture test exercises RAR through this helper"),
        }
    }

    fn extract(
        &self,
        archive: &Path,
        destination: &Path,
    ) -> Result<Option<String>, Box<dyn Error>> {
        Ok(completed_extract(self.run(archive, destination)?)?)
    }
}

/// Writes `entries` as regular files with each format's plain writer.
fn write_files(
    archive: &Path,
    format: ArchiveFormat,
    entries: &[(&str, &[u8])],
) -> Result<(), Box<dyn Error>> {
    match format {
        ArchiveFormat::Zip => write_zip(archive, entries),
        ArchiveFormat::SevenZ => write_7z_entries(archive, entries),
        ArchiveFormat::Tar | ArchiveFormat::TarGz => {
            let entries: Vec<_> = entries
                .iter()
                .map(|(name, contents)| (tar::EntryType::Regular, *name, *contents))
                .collect();
            write_tar_entries(archive, &entries, format == ArchiveFormat::TarGz)
        }
        ArchiveFormat::Rar => unreachable!("RAR compression is not supported"),
    }
}

/// Pushes `entries` as one solid block or as separate streams.
fn push_7z_files(
    writer: &mut sevenz_rust2::ArchiveWriter<fs::File>,
    entries: &[(&str, &[u8])],
    solid: bool,
) -> Result<(), Box<dyn Error>> {
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
                Some(Cursor::new(*contents)),
            )?;
        }
    }
    Ok(())
}

/// Flips one bit of a stored `payload` in the archive and returns the damaged bytes.
fn corrupt_stored_payload(
    archive: &Path,
    payload: &[u8],
    flip: u8,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = fs::read(archive)?;
    let offset = bytes
        .windows(payload.len())
        .position(|window| window == payload)
        .ok_or("stored contents not found")?;
    bytes[offset] ^= flip;
    fs::write(archive, &bytes)?;
    Ok(bytes)
}

fn kept_in(name: &str) -> String {
    format!("{INVALID_ARCHIVE} Extracted entries remain in “{name}”.")
}

fn locations(directory: &Path, names: &[&str]) -> Vec<Location> {
    names
        .iter()
        .map(|name| Location::local(directory.join(name)))
        .collect()
}

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

/// The locations a cancelled run left untouched, after nothing completed or failed.
fn not_attempted_of(outcome: ArchiveOutcome<Option<String>>, context: &str) -> Vec<Location> {
    match outcome {
        ArchiveOutcome::Cancelled {
            completed,
            failed,
            not_attempted,
        } => {
            assert!(completed.is_empty(), "{context}: {completed:?}");
            assert!(failed.is_empty(), "{context}: {failed:?}");
            not_attempted
        }
        ArchiveOutcome::Completed(name) => {
            panic!("{context}: extraction continued after cancellation: {name:?}")
        }
    }
}

fn assert_failed_keeping(
    result: &Result<ArchiveOutcome<Option<String>>, ArchiveError>,
    name: &str,
    context: &str,
) {
    let kept = kept_in(name);
    assert!(
        matches!(result, Err(ArchiveError::Failed(message)) if *message == kept),
        "{context}: {result:?}"
    );
}

fn assert_empty(directory: &Path, context: &str) -> Result<(), Box<dyn Error>> {
    let names = names_in(directory)?;
    assert!(
        names.is_empty(),
        "{context}: `{}` holds {names:?}",
        directory.display()
    );
    Ok(())
}

fn assert_entry_count(directory: &Path, count: usize, context: &str) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        fs::read_dir(directory)?.count(),
        count,
        "{context}: entries of `{}`",
        directory.display()
    );
    Ok(())
}

fn assert_contents(
    directory: &Path,
    files: &[(&str, &[u8])],
    context: &str,
) -> Result<(), Box<dyn Error>> {
    for (name, contents) in files {
        assert_eq!(
            fs::read(directory.join(name))?,
            *contents,
            "{context}: contents of `{name}`"
        );
    }
    Ok(())
}

fn assert_symlink(
    path: &Path,
    target: impl AsRef<Path>,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let target = target.as_ref();
    assert!(
        fs::symlink_metadata(path)?.file_type().is_symlink(),
        "{context}: `{}` extracted as {}, not as a symlink to `{}`",
        path.display(),
        kind_of(path),
        target.display()
    );
    assert_eq!(fs::read_link(path)?, target, "{context}");
    Ok(())
}

fn assert_hard_link(link: &Path, original: &Path, context: &str) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        fs::metadata(link)?.ino(),
        fs::metadata(original)?.ino(),
        "{context}: `{}` extracted as {}, not as a hard link to `{}`",
        link.display(),
        kind_of(link),
        original.display()
    );
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
