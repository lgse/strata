// SPDX-License-Identifier: GPL-3.0-or-later

//! ZIP, TAR/gzip and 7z decoding adapters feeding the same extraction session.
//! Format-specific member enumeration, passwords and error translation stay here.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};

use crate::model::Location;

use super::{
    ARCHIVE_CANCELLED, ArchiveError, archive_failed,
    destination::validated_archive_path,
    extraction::{ArchiveOutcome, ExtractionSession, MemberContent, extract_entry_location},
};

#[cfg(test)]
mod tests;

fn sevenz_error(error: ArchiveError) -> sevenz_rust2::Error {
    sevenz_rust2::Error::Other(error.to_string().into())
}

fn sevenz_is_cancelled(error: &sevenz_rust2::Error) -> bool {
    matches!(error, sevenz_rust2::Error::Other(message) if message.as_ref() == ARCHIVE_CANCELLED)
}

fn zip_entry_locations(
    archive: &zip::ZipArchive<std::fs::File>,
    destination: &Path,
    from: usize,
) -> Vec<Location> {
    (from..archive.len())
        .filter_map(|index| archive.name_for_index(index))
        .map(|name| Location::local(destination.join(name)))
        .collect()
}

fn tar_entry_location<'a, R: std::io::Read + 'a>(
    entry: tar::Entry<'a, R>,
    dest_dir: &Path,
) -> Option<Location> {
    let name = entry.path().ok()?;
    let path = validated_archive_path(&name.to_string_lossy()).ok()?;
    Some(extract_entry_location(dest_dir, &path))
}

fn sevenz_locations_from(
    dest_dir: &Path,
    names: &[String],
    from_name: &str,
    skip_current: bool,
) -> Vec<Location> {
    let Some(index) = names.iter().position(|name| name == from_name) else {
        return Vec::new();
    };
    let start = if skip_current { index + 1 } else { index };
    names
        .get(start..)
        .unwrap_or(&[])
        .iter()
        .filter_map(|name| validated_archive_path(name).ok())
        .map(|path| extract_entry_location(dest_dir, &path))
        .collect()
}

pub(super) fn extract_zip_from_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let pw_bytes = password.map(str::as_bytes);
    let mut next_index = 0;
    let result = (|| {
        for index in 0..archive.len() {
            session.check_cancelled()?;
            let options = zip::read::ZipReadOptions::new().password(pw_bytes);
            let mut entry = archive
                .by_index_with_options(index, options)
                .map_err(archive_failed)?;
            let name = entry.name().to_owned();
            entry
                .enclosed_name()
                .ok_or_else(|| format!("Refusing unsafe ZIP path: {name}"))?;
            let content = if entry.is_dir() {
                MemberContent::Directory
            } else {
                MemberContent::File(&mut entry)
            };
            next_index = index + 1;
            session.extract_member(&name, content)?;
        }
        Ok(())
    })();
    session.finish(result, || {
        zip_entry_locations(archive, dest_dir, next_index)
    })
}

/// TAR cancellation reports at most the current member, never scans unread ones.
pub(super) fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    gzip: bool,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let file = std::fs::File::open(archive_path).map_err(archive_failed)?;
    let reader: Box<dyn std::io::Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut remaining = None;
    let result = (|| {
        for entry in archive.entries().map_err(archive_failed)? {
            if let Err(error) = session.check_cancelled() {
                remaining = entry
                    .ok()
                    .and_then(|entry| tar_entry_location(entry, dest_dir));
                return Err(error);
            }
            let mut entry = entry.map_err(archive_failed)?;
            let name = entry.path().map_err(archive_failed)?;
            let directory = entry.header().entry_type().is_dir();
            if directory && name == Path::new(".") {
                continue;
            }
            let name = name.to_string_lossy().into_owned();
            let content = if directory {
                MemberContent::Directory
            } else {
                MemberContent::File(&mut entry)
            };
            session.extract_member(&name, content)?;
        }
        Ok(())
    })();
    session.finish(result, || remaining.into_iter().collect())
}

pub(super) fn extract_7z_from_reader(
    reader: std::fs::File,
    dest_dir: &Path,
    password: sevenz_rust2::Password,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let mut archive = sevenz_rust2::ArchiveReader::new(reader, password).map_err(archive_failed)?;
    let entry_names: Vec<String> = archive
        .archive()
        .files
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let mut remaining = None;
    let result = archive.for_each_entries(|entry, reader| {
        if let Err(error) = session.check_cancelled() {
            remaining = Some((entry.name.clone(), false));
            return Err(sevenz_error(error));
        }
        let content = if entry.is_directory {
            MemberContent::Directory
        } else {
            MemberContent::File(reader)
        };
        if let Err(error) = session.extract_member(&entry.name, content) {
            if error == ArchiveError::Cancelled {
                remaining = Some((entry.name.clone(), true));
            }
            return Err(sevenz_error(error));
        }
        Ok(true)
    });
    let result = result.map_err(|error| {
        if sevenz_is_cancelled(&error) {
            ArchiveError::Cancelled
        } else {
            archive_failed(error)
        }
    });
    session.finish(result, || {
        remaining.map_or_else(Vec::new, |(name, skip_current)| {
            sevenz_locations_from(dest_dir, &entry_names, &name, skip_current)
        })
    })
}
