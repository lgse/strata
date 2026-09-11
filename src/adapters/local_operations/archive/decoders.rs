// SPDX-License-Identifier: MIT

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

const INVALID_ARCHIVE: &str = "This file is not a valid archive or is damaged.";
// Plain-header 7z and ZipCrypto cannot distinguish a wrong password from a content checksum failure.
const MAYBE_BAD_PASSWORD: &str = "The password may be incorrect.";

pub(super) fn zip_error(error: zip::result::ZipError) -> ArchiveError {
    match error {
        zip::result::ZipError::InvalidArchive(_) => archive_failed(INVALID_ARCHIVE),
        zip::result::ZipError::Io(error) => archive_failed(archive_read_error(error, false)),
        error => archive_failed(error),
    }
}

fn sevenz_decode_error(error: sevenz_rust2::Error) -> ArchiveError {
    use sevenz_rust2::Error;
    match error {
        Error::BadSignature(_)
        | Error::ChecksumVerificationFailed
        | Error::NextHeaderCrcMismatch
        | Error::BadTerminatedStreamsInfo(_)
        | Error::BadTerminatedUnpackInfo
        | Error::BadTerminatedPackInfo(_)
        | Error::BadTerminatedSubStreamsInfo
        | Error::BadTerminatedHeader(_) => archive_failed(INVALID_ARCHIVE),
        Error::PasswordRequired => {
            archive_failed("A password is required to extract this archive.")
        }
        Error::MaybeBadPassword(_) => archive_failed("The password may be incorrect."),
        Error::Other(message) if message.as_ref() == INVALID_ARCHIVE => archive_failed(message),
        Error::Other(message) if message.as_ref() == MAYBE_BAD_PASSWORD => archive_failed(message),
        Error::Io(error, _) => archive_failed(archive_read_error(error, false)),
        error => archive_failed(error),
    }
}

fn unrar_decode_error(error: unrar::error::UnrarError, password_supplied: bool) -> ArchiveError {
    use unrar::error::Code;
    match error.code {
        Code::MissingPassword => archive_failed("A password is required to extract this archive."),
        Code::BadPassword => archive_failed(MAYBE_BAD_PASSWORD),
        Code::BadData if password_supplied => archive_failed(MAYBE_BAD_PASSWORD),
        Code::BadArchive | Code::UnknownFormat | Code::BadData => archive_failed(INVALID_ARCHIVE),
        Code::EOpen | Code::ERead | Code::EClose => archive_failed(archive_read_error(
            std::io::Error::other(error.to_string()),
            password_supplied,
        )),
        _ => archive_failed(error),
    }
}

fn archive_read_error(error: std::io::Error, password_supplied: bool) -> std::io::Error {
    use std::io::ErrorKind;
    let checksum_failed = matches!(
        error
            .get_ref()
            .and_then(|error| error.downcast_ref::<sevenz_rust2::Error>()),
        Some(sevenz_rust2::Error::ChecksumVerificationFailed)
    );
    if password_supplied
        && (matches!(
            error.kind(),
            ErrorKind::InvalidData | ErrorKind::UnexpectedEof | ErrorKind::InvalidInput
        ) || checksum_failed)
    {
        return std::io::Error::new(ErrorKind::InvalidData, MAYBE_BAD_PASSWORD);
    }
    // TAR reports these malformed-header errors as Other, not InvalidData.
    let invalid_tar = error.kind() == ErrorKind::Other
        && matches!(
            error.to_string().as_str(),
            "failed to read entire block"
                | "archive header checksum mismatch"
                | "unexpected EOF during skip"
        );
    let invalid_gzip = error.kind() == ErrorKind::InvalidInput
        && matches!(
            error.to_string().as_str(),
            "invalid gzip header"
                | "corrupt gzip stream does not have a matching checksum"
                | "gzip header field too long"
                | "corrupt deflate stream"
        );
    if matches!(
        error.kind(),
        ErrorKind::InvalidData | ErrorKind::UnexpectedEof
    ) || checksum_failed
        || invalid_tar
        || invalid_gzip
    {
        std::io::Error::new(ErrorKind::InvalidData, INVALID_ARCHIVE)
    } else {
        error
    }
}

// Translate only decoder reads; destination writes retain their own errors.
struct ArchiveReader<R> {
    inner: R,
    password_supplied: bool,
}

impl<R> ArchiveReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            password_supplied: false,
        }
    }
}

impl<R: Read> Read for ArchiveReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner
            .read(buffer)
            .map_err(|error| archive_read_error(error, self.password_supplied))
    }
}

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
    if let Some(claimed) = archive.decompressed_size() {
        session.preflight_claimed_size(claimed)?;
    }
    let password_supplied = password.is_some();
    let pw_bytes = password.map(str::as_bytes);
    let mut next_index = 0;
    let result = (|| {
        for index in 0..archive.len() {
            session.check_cancelled()?;
            let options = zip::read::ZipReadOptions::new().password(pw_bytes);
            let mut entry = archive
                .by_index_with_options(index, options)
                .map_err(zip_error)?;
            let name = entry.name().to_owned();
            entry
                .enclosed_name()
                .ok_or_else(|| format!("Refusing unsafe ZIP path: {name}"))?;
            let declared_size = entry.size();
            let directory = entry.is_dir();
            let mut reader = ArchiveReader {
                inner: &mut entry,
                password_supplied,
            };
            let content = if directory {
                MemberContent::Directory
            } else {
                MemberContent::File(&mut reader, Some(declared_size))
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
        for entry in archive
            .entries()
            .map_err(|error| archive_failed(archive_read_error(error, false)))?
        {
            if let Err(error) = session.check_cancelled() {
                remaining = entry.ok().and_then(|entry| {
                    entry
                        .path()
                        .ok()
                        .map(|name| name.to_string_lossy().into_owned())
                });
                return Err(error);
            }
            let mut entry =
                entry.map_err(|error| archive_failed(archive_read_error(error, false)))?;
            // tar-rs consumes per-entry extended headers itself, but a pax
            // global header (the first member of every `git archive` tarball)
            // is yielded as an ordinary entry. It carries no file.
            if matches!(
                entry.header().entry_type(),
                tar::EntryType::XGlobalHeader
                    | tar::EntryType::XHeader
                    | tar::EntryType::GNULongName
                    | tar::EntryType::GNULongLink
            ) {
                continue;
            }
            let name = entry.path().map_err(archive_failed)?;
            let directory = entry.header().entry_type().is_dir();
            if directory && name == Path::new(".") {
                continue;
            }
            let declared_size = entry.size();
            let name = name.to_string_lossy().into_owned();
            let mut reader = ArchiveReader::new(&mut entry);
            let content = if directory {
                MemberContent::Directory
            } else {
                MemberContent::File(&mut reader, Some(declared_size))
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
    let password_supplied = !password.is_empty();
    let mut archive =
        sevenz_rust2::ArchiveReader::new(reader, password).map_err(sevenz_decode_error)?;
    let claimed = archive
        .archive()
        .files
        .iter()
        .try_fold(0u128, |total, entry| {
            total.checked_add(u128::from(entry.size))
        });
    if let Some(claimed) = claimed {
        session.preflight_claimed_size(claimed)?;
    }
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
        let mut reader = ArchiveReader {
            inner: reader,
            password_supplied,
        };
        let content = if entry.is_directory {
            MemberContent::Directory
        } else {
            MemberContent::File(&mut reader, Some(entry.size))
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
            sevenz_decode_error(error)
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

pub(super) fn extract_rar(
    archive_path: &Path,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let password_supplied = password.is_some();
    let mut remaining = Vec::new();
    let result = (|| -> Result<(), ArchiveError> {
        let mut open_archive = match password {
            Some(pw) => unrar::Archive::with_password(archive_path, pw),
            None => unrar::Archive::new(archive_path),
        }
        .open_for_processing()
        .map_err(|e| unrar_decode_error(e, password_supplied))?;

        while let Some(file_archive) = open_archive
            .read_header()
            .map_err(|e| unrar_decode_error(e, password_supplied))?
        {
            if let Err(error) = session.check_cancelled() {
                remaining.push(file_archive.entry().filename.to_string_lossy().into_owned());
                return Err(error);
            }
            let header = file_archive.entry();
            let name = header.filename.to_string_lossy().into_owned();
            let is_dir = header.is_directory();
            let unpacked_size = header.unpacked_size;

            if is_dir {
                session.extract_member(&name, MemberContent::Directory)?;
                open_archive = file_archive
                    .skip()
                    .map_err(|e| unrar_decode_error(e, password_supplied))?;
            } else {
                let (bytes, next_archive) = file_archive
                    .read()
                    .map_err(|e| unrar_decode_error(e, password_supplied))?;
                open_archive = next_archive;
                let mut cursor = std::io::Cursor::new(bytes);
                session
                    .extract_member(&name, MemberContent::File(&mut cursor, Some(unpacked_size)))?;
            }
        }
        Ok(())
    })();
    session.finish(result, || remaining)
}
