// SPDX-License-Identifier: MIT

//! ZIP, TAR/gzip and 7z decoding adapters feeding the same extraction session.
//! Format-specific member enumeration, passwords and error translation stay here.
//!

use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::OsStr,
    io::{BufRead, BufReader, Read, Seek},
    os::unix::ffi::OsStrExt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gtk::glib;

use crate::services::INCORRECT_ARCHIVE_PASSWORD;

use super::{
    ArchiveError, MAYBE_BAD_PASSWORD, PASSWORD_REQUIRED, archive_failed, archive_io_failed,
    archive_read_failed, check_archive_cancelled, copy_with_big_buf,
    extraction::{
        ArchiveOutcome, ExtractionSession, MAX_SYMLINK_TARGET_BYTES, MemberContent, MemberMetadata,
    },
};

#[cfg(feature = "rar")]
mod rar;
#[cfg(feature = "rar")]
pub(super) use rar::extract_rar;

#[cfg(test)]
mod tests;

const INVALID_ARCHIVE: &str = "This file is not a valid archive or is damaged.";
const UNSUPPORTED_ARCHIVE: &str =
    "This archive uses a compression method or feature that Strata does not support.";

fn invalid_archive() -> ArchiveError {
    archive_failed(crate::i18n::tr(INVALID_ARCHIVE))
}

pub(super) fn zip_error(error: zip::result::ZipError) -> ArchiveError {
    use zip::result::ZipError;
    match error {
        ZipError::InvalidArchive(_) => invalid_archive(),
        ZipError::Io(error) => archive_io_failed(&archive_read_error(error, false)),
        // ZIP's password check is a definite rejection, unlike a failed CRC or HMAC.
        ZipError::InvalidPassword => {
            ArchiveError::IncorrectPassword(INCORRECT_ARCHIVE_PASSWORD.to_owned())
        }
        ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => {
            ArchiveError::PasswordRequired(PASSWORD_REQUIRED.to_owned())
        }
        error @ (ZipError::UnsupportedArchive(_) | ZipError::CompressionMethodNotSupported(_)) => {
            unsupported_archive(&error)
        }
        error => {
            tracing::warn!(%error, "unexpected ZIP decoder error");
            invalid_archive()
        }
    }
}

fn unsupported_archive(error: &dyn std::fmt::Display) -> ArchiveError {
    tracing::warn!(%error, "unsupported archive feature");
    archive_failed(crate::i18n::tr(UNSUPPORTED_ARCHIVE))
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
        | Error::BadTerminatedHeader(_) => invalid_archive(),
        Error::PasswordRequired => ArchiveError::PasswordRequired(PASSWORD_REQUIRED.to_owned()),
        Error::MaybeBadPassword(_) => {
            ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_owned())
        }
        Error::Io(error, _) => archive_io_failed(&archive_read_error(error, false)),
        Error::FileOpen(error, _) => archive_io_failed(&error),
        error @ (Error::UnsupportedCompressionMethod(_)
        | Error::Unsupported(_)
        | Error::ExternalUnsupported
        | Error::UnsupportedVersion { .. }) => unsupported_archive(&error),
        error => {
            tracing::warn!(%error, "unexpected 7z decoder error");
            invalid_archive()
        }
    }
}

/// Only encrypted data permits password retry; plain-header 7z and ZipCrypto
/// cannot distinguish a wrong password from damage.
fn archive_read_error(error: std::io::Error, decrypting: bool) -> std::io::Error {
    use std::io::ErrorKind;
    let checksum_failed = matches!(
        error
            .get_ref()
            .and_then(|error| error.downcast_ref::<sevenz_rust2::Error>()),
        Some(sevenz_rust2::Error::ChecksumVerificationFailed)
    );
    if decrypting
        && (matches!(
            error.kind(),
            ErrorKind::InvalidData | ErrorKind::UnexpectedEof | ErrorKind::InvalidInput
        ) || checksum_failed)
    {
        return std::io::Error::new(
            ErrorKind::InvalidData,
            ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_owned()),
        );
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
        std::io::Error::new(ErrorKind::InvalidData, crate::i18n::tr(INVALID_ARCHIVE))
    } else {
        error
    }
}

/// 7z attribute bit with which p7zip marks `st_mode` stored in the upper 16 bits.
pub(super) const FILE_ATTRIBUTE_UNIX_EXTENSION: u32 = 0x8000;
const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

/// ZIP/7z may report zero or junk modes; don't revoke access based on them.
fn member_mode(mode: Option<u32>, directory: bool) -> Option<u32> {
    let kind = if directory { S_IFDIR } else { S_IFREG };
    mode.filter(|mode| mode & S_IFMT == kind)
}

fn unix_seconds(seconds: impl TryInto<u64>) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds.try_into().ok()?))
}

/// FILETIME zero denotes an absent timestamp.
fn filetime(value: u64) -> Option<SystemTime> {
    (value != 0)
        .then(|| SystemTime::from(sevenz_rust2::NtTime::from(value)))
        .filter(|time| *time >= UNIX_EPOCH)
}

/// DOS times carry no zone and are read as local time, like `unzip`.
fn dos_local_time(time: zip::DateTime) -> Option<SystemTime> {
    let local = glib::DateTime::from_local(
        time.year().into(),
        time.month().into(),
        time.day().into(),
        time.hour().into(),
        time.minute().into(),
        time.second().into(),
    )
    .ok()?;
    unix_seconds(local.to_unix())
}

fn zip_member_modified(entry: &zip::read::ZipFile<'_, std::fs::File>) -> Option<SystemTime> {
    let extended = entry.extra_data_fields().find_map(|field| match field {
        // The field is signed; times before 1970 are skipped.
        zip::ExtraField::ExtendedTimestamp(stamp) => stamp
            .mod_time()
            .and_then(|seconds| unix_seconds(seconds as i32)),
        zip::ExtraField::Ntfs(_) => None,
    });
    let ntfs = || {
        entry.extra_data_fields().find_map(|field| match field {
            zip::ExtraField::Ntfs(ntfs) => filetime(ntfs.mtime()),
            zip::ExtraField::ExtendedTimestamp(_) => None,
        })
    };
    extended.or_else(ntfs).or_else(|| {
        entry
            .last_modified()
            .filter(zip::DateTime::is_valid)
            .and_then(dos_local_time)
    })
}

/// Read one extra byte so overlong targets cannot be silently truncated.
fn read_link_target(reader: &mut impl Read) -> Result<Vec<u8>, ArchiveError> {
    let mut target = Vec::new();
    reader
        .take(MAX_SYMLINK_TARGET_BYTES + 1)
        .read_to_end(&mut target)
        .map_err(archive_read_failed)?;
    Ok(target)
}

// Translate only decoder reads; destination writes retain their own errors.
struct ArchiveReader<R> {
    inner: R,
    decrypting: bool,
}

impl<R> ArchiveReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            decrypting: false,
        }
    }
}

impl<R: Read> Read for ArchiveReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner
            .read(buffer)
            .map_err(|error| archive_read_error(error, self.decrypting))
    }
}

const GZIP_MAGIC: u8 = 0x1f;

/// Unlike MultiGzDecoder, accepts zero padding after the final member.
struct GzipMembers<R> {
    /// Always `Some` outside a transition in [`Read::read`].
    state: Option<GzipState<R>>,
}

enum GzipState<R> {
    Member(Box<flate2::bufread::GzDecoder<R>>),
    /// Retain this state on read errors so a retry cannot mistake them for EOF.
    Boundary(R),
    Rest(R),
}

impl<R: BufRead> GzipMembers<R> {
    fn new(reader: R) -> Self {
        Self {
            state: Some(GzipState::Member(Box::new(
                flate2::bufread::GzDecoder::new(reader),
            ))),
        }
    }

    /// Tape blocking and `dd` may leave zero padding after the gzip trailer.
    fn verify_padding(self, cancelled: &AtomicBool) -> Result<(), ArchiveError> {
        let Some(GzipState::Rest(mut rest)) = self.state else {
            return Ok(());
        };
        loop {
            check_archive_cancelled(cancelled)?;
            let chunk = rest
                .fill_buf()
                .map_err(|error| archive_read_failed(archive_read_error(error, false)))?;
            if chunk.is_empty() {
                return Ok(());
            }
            if chunk.iter().any(|byte| *byte != 0) {
                return Err(invalid_archive());
            }
            let length = chunk.len();
            rest.consume(length);
        }
    }
}

impl<R: BufRead> Read for GzipMembers<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let next_member = match self.state.as_mut().expect("gzip state is present") {
                GzipState::Member(decoder) => {
                    let count = decoder.read(buffer)?;
                    if count > 0 {
                        return Ok(count);
                    }
                    false
                }
                GzipState::Boundary(inner) => inner.fill_buf()?.first() == Some(&GZIP_MAGIC),
                GzipState::Rest(_) => return Ok(0),
            };
            let state = match self.state.take().expect("gzip state is present") {
                GzipState::Member(decoder) => GzipState::Boundary((*decoder).into_inner()),
                GzipState::Boundary(inner) if next_member => {
                    GzipState::Member(Box::new(flate2::bufread::GzDecoder::new(inner)))
                }
                GzipState::Boundary(inner) | GzipState::Rest(inner) => GzipState::Rest(inner),
            };
            let ended = matches!(state, GzipState::Rest(_));
            self.state = Some(state);
            if ended {
                return Ok(0);
            }
        }
    }
}

/// tar-rs stops before gzip's CRC32/ISIZE trailer; drain every member to verify it.
fn verify_gzip_trailer<R: BufRead>(
    mut members: GzipMembers<R>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    copy_with_big_buf(
        ArchiveReader::new(&mut members),
        &mut std::io::sink(),
        cancelled,
    )?;
    members.verify_padding(cancelled)
}

pub(super) fn extract_zip_from_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    dest_dir: &Path,
    archive_name: &str,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, archive_name, progress, cancelled)?;
    if let Some(claimed) = archive.decompressed_size() {
        session.preflight_claimed_size(claimed)?;
    }
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
            let declared_size = entry.size();
            let directory = entry.is_dir();
            let symlink = !directory && entry.is_symlink();
            let metadata = MemberMetadata {
                mode: member_mode(entry.unix_mode(), directory),
                modified: zip_member_modified(&entry),
            };
            let decrypting = password.is_some() && entry.encrypted();
            let mut reader = ArchiveReader {
                inner: &mut entry,
                decrypting,
            };
            let target;
            let content = if directory {
                MemberContent::Directory
            } else if symlink {
                target = read_link_target(&mut reader)?;
                MemberContent::Symlink(&target)
            } else {
                MemberContent::File(&mut reader, Some(declared_size))
            };
            next_index = index + 1;
            session.extract_member(&name, content, metadata)?;
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

enum TarInput {
    Plain(std::fs::File),
    Gzip(GzipMembers<BufReader<std::fs::File>>),
}

impl Read for TarInput {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(file) => file.read(buffer),
            Self::Gzip(members) => members.read(buffer),
        }
    }
}

/// TAR cancellation reports at most the current member, never scans unread ones.
pub(super) fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    archive_name: &str,
    gzip: bool,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, archive_name, progress, cancelled)?;
    session.record_hard_link_targets();
    let file = std::fs::File::open(archive_path).map_err(|error| archive_io_failed(&error))?;
    let reader = if gzip {
        TarInput::Gzip(GzipMembers::new(BufReader::with_capacity(32 * 1024, file)))
    } else {
        TarInput::Plain(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut remaining = None;
    let result = (|| {
        for entry in archive
            .entries()
            .map_err(|error| archive_read_failed(archive_read_error(error, false)))?
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
                entry.map_err(|error| archive_read_failed(archive_read_error(error, false)))?;
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
            let name = entry.path().map_err(|error| archive_io_failed(&error))?;
            let entry_type = entry.header().entry_type();
            if entry_type.is_dir() && name == Path::new(".") {
                continue;
            }
            let declared_size = entry.size();
            let path = name.into_owned();
            let name = path.display();
            let header = entry.header();
            let metadata = MemberMetadata {
                mode: header.mode().ok(),
                modified: header.mtime().ok().and_then(unix_seconds),
            };
            let stored_link = entry.link_name_bytes().map(Cow::into_owned);
            let link_name = || {
                stored_link.as_deref().ok_or_else(|| {
                    archive_failed(rust_i18n::t!(
                        "Archive member “%{name}” has no link target",
                        name = name
                    ))
                })
            };
            let mut reader = ArchiveReader::new(&mut entry);
            let content = match entry_type {
                entry_type if entry_type.is_dir() => MemberContent::Directory,
                tar::EntryType::Symlink => MemberContent::Symlink(link_name()?),
                tar::EntryType::Link if declared_size == 0 => {
                    MemberContent::HardLink(Path::new(OsStr::from_bytes(link_name()?)))
                }
                tar::EntryType::Fifo => {
                    return Err(archive_failed(rust_i18n::t!(
                        "Archive member “%{name}” is a FIFO and cannot be extracted",
                        name = name
                    )));
                }
                tar::EntryType::Char | tar::EntryType::Block => {
                    return Err(archive_failed(rust_i18n::t!(
                        "Archive member “%{name}” is a device and cannot be extracted",
                        name = name
                    )));
                }
                _ => MemberContent::File(&mut reader, Some(declared_size)),
            };
            session.extract_member(&path, content, metadata)?;
        }
        Ok(())
    })();
    let result = result.and_then(|()| match archive.into_inner() {
        TarInput::Gzip(members) => verify_gzip_trailer(members, cancelled),
        TarInput::Plain(_) => Ok(()),
    });
    session.finish(result, || remaining.into_iter().collect())
}

fn encrypted_7z_members(archive: &sevenz_rust2::Archive) -> Vec<bool> {
    archive
        .stream_map
        .file_block_index
        .iter()
        .map(|block| {
            block
                .and_then(|block| archive.blocks.get(block))
                .is_some_and(|block| {
                    block.coders.iter().any(|coder| {
                        coder.encoder_method_id() == sevenz_rust2::EncoderMethod::ID_AES256_SHA256
                    })
                })
        })
        .collect()
}

pub(super) fn extract_7z_from_reader(
    reader: impl Read + Seek,
    dest_dir: &Path,
    archive_name: &str,
    password: sevenz_rust2::Password,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, archive_name, progress, cancelled)?;
    let password_supplied = !password.is_empty();
    let mut archive =
        sevenz_rust2::ArchiveReader::new(reader, password).map_err(sevenz_decode_error)?;
    let decrypting = encrypted_7z_members(archive.archive())
        .into_iter()
        .map(|encrypted| password_supplied && encrypted)
        .collect::<Vec<_>>();
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
    // sevenz_rust2 carries callback errors as text; retain their structured kind separately.
    let mut member_error = None;
    let result = archive.for_each_entries(|entry, reader| {
        let extracted = (|| {
            session.check_cancelled()?;
            let Some(&index) = member_indices.get(&std::ptr::from_ref(entry)) else {
                return Err(archive_failed("7z decoder returned an unknown member"));
            };
            let mut reader = ArchiveReader {
                inner: reader,
                decrypting: decrypting[index],
            };
            let unix_mode = (entry.has_windows_attributes
                && entry.windows_attributes & FILE_ATTRIBUTE_UNIX_EXTENSION != 0)
                .then_some(entry.windows_attributes >> 16);
            let symlink =
                !entry.is_directory && unix_mode.is_some_and(|mode| mode & S_IFMT == S_IFLNK);
            let metadata = MemberMetadata {
                mode: member_mode(unix_mode, entry.is_directory),
                modified: entry
                    .has_last_modified_date
                    .then(|| filetime(entry.last_modified_date.into()))
                    .flatten(),
            };
            let target;
            let content = if entry.is_directory {
                MemberContent::Directory
            } else if symlink {
                target = read_link_target(&mut reader)?;
                MemberContent::Symlink(&target)
            } else {
                MemberContent::File(&mut reader, Some(entry.size))
            };
            submitted[index] = true;
            session.extract_member(&entry.name, content, metadata)
        })();
        extracted.map(|()| true).map_err(|error| {
            let returned = sevenz_rust2::Error::Other(error.to_string().into());
            member_error = Some(error);
            returned
        })
    });
    let result = result.map_err(|error| {
        member_error
            .take()
            .unwrap_or_else(|| sevenz_decode_error(error))
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
