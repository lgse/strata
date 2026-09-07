// SPDX-License-Identifier: GPL-3.0-or-later

//! ZIP, TAR/gzip and 7z decoding adapters feeding the same extraction session.
//! Format-specific member enumeration, passwords and error translation stay here.
//!
//! This boundary preserves legacy output semantics: TAR names use lossy UTF-8
//! conversion, and non-directory entries (including links) become regular files.
//! Native names and richer entry types remain part of the decoder evaluation.

use std::{
    collections::HashMap,
    io::{Read, Seek},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};

use super::{
    ARCHIVE_CANCELLED, ArchiveError, archive_failed,
    extraction::{ArchiveOutcome, ExtractionSession, MemberContent},
};

#[cfg(test)]
mod tests;

fn sevenz_error(error: ArchiveError) -> sevenz_rust2::Error {
    sevenz_rust2::Error::Other(error.to_string().into())
}

fn sevenz_is_cancelled(error: &sevenz_rust2::Error) -> bool {
    matches!(error, sevenz_rust2::Error::Other(message) if message.as_ref() == ARCHIVE_CANCELLED)
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
        (next_index..archive.len())
            .filter_map(|index| archive.name_for_index(index))
            .map(str::to_owned)
            .collect()
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
                remaining = entry.ok().and_then(|entry| {
                    entry
                        .path()
                        .ok()
                        .map(|name| name.to_string_lossy().into_owned())
                });
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
    reader: impl Read + Seek,
    dest_dir: &Path,
    password: sevenz_rust2::Password,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let mut archive = sevenz_rust2::ArchiveReader::new(reader, password).map_err(archive_failed)?;
    // for_each_entries lends elements of the unchanged header vector, but visits
    // them out of order. Addresses identify even duplicate names; never dereference
    // these keys, and keep them local to this reader invocation.
    let member_indices: HashMap<_, _> = archive
        .archive()
        .files
        .iter()
        .enumerate()
        .map(|(index, entry)| (std::ptr::from_ref(entry), index))
        .collect();
    let mut submitted = vec![false; member_indices.len()];
    let result = archive.for_each_entries(|entry, reader| {
        session.check_cancelled().map_err(sevenz_error)?;
        let Some(&index) = member_indices.get(&std::ptr::from_ref(entry)) else {
            return Err(sevenz_rust2::Error::Other(
                "7z decoder returned an unknown member".into(),
            ));
        };
        let content = if entry.is_directory {
            MemberContent::Directory
        } else {
            MemberContent::File(reader)
        };
        submitted[index] = true;
        session
            .extract_member(&entry.name, content)
            .map_err(sevenz_error)?;
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
        archive
            .archive()
            .files
            .iter()
            .enumerate()
            .filter(|(index, _)| !submitted[*index])
            .map(|(_, entry)| entry.name.clone())
            .collect()
    })
}
