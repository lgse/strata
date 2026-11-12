// SPDX-License-Identifier: MIT
//! The private host directory a renderer writes into, and validation of what it wrote.

use std::{
    ffi::OsStr,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Once,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use super::{ParseOperation, metadata, raw_metadata};

pub(super) static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

fn fits_image_limits(operation: &ParseOperation, data: &[u8]) -> bool {
    let Some((width, height)) = png_dimensions(data) else {
        return false;
    };
    let Some((max_width, max_height, max_pixels)) = operation.image_limits() else {
        return false;
    };
    let within_edges = width <= max_width && height <= max_height;
    within_edges
        && u64::from(width)
            .checked_mul(u64::from(height))
            .is_some_and(|pixels| pixels <= max_pixels)
}

pub(super) fn valid_output(operation: ParseOperation, data: &[u8]) -> bool {
    match operation {
        ParseOperation::PreviewRoute(_) => {
            crate::services::route_preview::RoutePreview::from_json(data).is_ok()
        }
        ParseOperation::InspectImage => {
            data.len() <= 32
                && serde_json::from_slice::<crate::services::image_conversion::ImageKind>(data)
                    .is_ok()
        }
        ParseOperation::AudioCover if data == b"null" => true,
        ParseOperation::PreviewWorkbook => {
            crate::services::table::TableData::from_json(data).is_ok()
        }
        ParseOperation::PreviewDocument(_) => {
            crate::services::docx::RichTextData::from_json(data).is_ok()
        }
        ParseOperation::RawMetadata => raw_metadata::RawMetadata::from_json(data).is_ok(),
        ParseOperation::AudioTags => metadata::AudioTags::from_json(data).is_ok(),
        ParseOperation::MediaMetadata => {
            data.len() as u64 <= metadata::MAX_METADATA_BYTES
                && serde_json::from_slice::<serde_json::Value>(data).is_ok()
        }
        ParseOperation::ArchiveList { .. } => {
            crate::sandbox::archive_listing::archive_payload_valid(data)
        }
        _ if operation.is_media() => false,
        _ => fits_image_limits(&operation, data),
    }
}

pub fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let signed = data.starts_with(b"\x89PNG\r\n\x1a\n");
    if !signed || data.get(8..12)? != 13u32.to_be_bytes() {
        return None;
    }
    if data.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

pub(super) fn read_private_output(path: &Path, max_bytes: u64) -> Result<Vec<u8>, OutputError> {
    use rustix::fs::{FileType, Mode, OFlags, fstat, open};

    // The renderer controls the final entry, but not the host directory ancestors.
    // NONBLOCK lets us reject a FIFO without waiting for a writer at open time.
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| OutputError::Missing)?;
    let stat = fstat(&fd).map_err(|error| OutputError::Io(error.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(OutputError::NonRegular);
    }
    let len = u64::try_from(stat.st_size).unwrap_or(0);
    if len == 0 {
        return Err(OutputError::Empty);
    }
    if len > max_bytes {
        return Err(OutputError::Oversize);
    }
    let mut data = Vec::new();
    fs::File::from(fd)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(|error| OutputError::Io(error.to_string()))?;
    if data.is_empty() {
        return Err(OutputError::Empty);
    }
    if data.len() as u64 > max_bytes {
        return Err(OutputError::Oversize);
    }
    Ok(data)
}

#[derive(Debug, PartialEq)]
pub(super) enum OutputError {
    Missing,
    NonRegular,
    Empty,
    Oversize,
    Io(String),
}

impl OutputError {
    pub(super) fn message(&self) -> String {
        match self {
            Self::Missing => "The preview renderer produced no output".to_owned(),
            Self::NonRegular => "The preview renderer produced a non-regular output".to_owned(),
            Self::Empty | Self::Oversize => {
                "The preview renderer produced an invalid output size".to_owned()
            }
            Self::Io(message) => message.clone(),
        }
    }
}

pub(super) fn archive_output_error(error: &OutputError) -> String {
    match error {
        OutputError::Oversize => {
            crate::sandbox::archive_listing::ARCHIVE_TOO_LARGE_MESSAGE.to_owned()
        }
        _ => crate::sandbox::archive_listing::INVALID_ARCHIVE.to_owned(),
    }
}

pub(super) fn read_metadata(path: &Path) -> (i32, i32) {
    let Ok(bytes) = read_private_output(path, 256) else {
        return (0, 0);
    };
    let Ok(value) = std::str::from_utf8(&bytes) else {
        return (0, 0);
    };
    let mut values = value
        .split_whitespace()
        .filter_map(|part| part.parse().ok());
    (values.next().unwrap_or(0), values.next().unwrap_or(0))
}

const PRIVATE_OUTPUT_PREFIX: &str = "strata-preview-";

const PRIVATE_OUTPUT_ATTEMPTS: usize = 64;

// The flock on the directory marks it as in use; the sweep removes only directories it can
// lock, which also works across pid namespaces that share the temp directory.
pub(super) struct PrivateOutput {
    path: PathBuf,
    _lock: rustix::fd::OwnedFd,
}

impl PrivateOutput {
    pub(super) fn create() -> io::Result<Self> {
        static SWEEP: Once = Once::new();
        SWEEP.call_once(|| {
            thread::spawn(|| prune_orphaned_private_outputs(&std::env::temp_dir()));
        });
        Self::create_in(&std::env::temp_dir())
    }

    pub(super) fn create_in(parent: &Path) -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;

        for _ in 0..PRIVATE_OUTPUT_ATTEMPTS {
            let path = parent.join(format!(
                "{PRIVATE_OUTPUT_PREFIX}{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                result => result?,
            }
            if let Some(lock) = lock_new_private_output(&path)? {
                return Ok(Self { path, _lock: lock });
            }
        }
        Err(io::Error::from(io::ErrorKind::AlreadyExists))
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateOutput {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.path);
    }
}

fn open_private_output(path: &Path) -> rustix::io::Result<rustix::fd::OwnedFd> {
    use rustix::fs::{Mode, OFlags, open};

    open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

// `None` means a concurrent sweep claimed the new directory before it was locked.
fn lock_new_private_output(path: &Path) -> io::Result<Option<rustix::fd::OwnedFd>> {
    use rustix::fs::{FlockOperation, flock, fstat, lstat};

    let fd = match open_private_output(path) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    // Other lock errors are ignored: without lock support the sweep cannot lock it either.
    if let Err(rustix::io::Errno::WOULDBLOCK) = flock(&fd, FlockOperation::NonBlockingLockExclusive)
    {
        return Ok(None);
    }
    let locked = fstat(&fd)?;
    let same = lstat(path)
        .is_ok_and(|current| (current.st_dev, current.st_ino) == (locked.st_dev, locked.st_ino));
    Ok(same.then_some(fd))
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_private_output_name(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix(PRIVATE_OUTPUT_PREFIX))
        .and_then(|rest| rest.split_once('-'))
        .is_some_and(|(pid, sequence)| all_digits(pid) && all_digits(sequence))
}

/// Removes a directory this user owns once no live output holds its lock.
fn remove_if_orphaned(path: &Path, uid: u32) {
    use rustix::fs::{FlockOperation, flock, fstat};

    let Ok(fd) = open_private_output(path) else {
        return;
    };
    let ours = fstat(&fd).is_ok_and(|stat| stat.st_uid == uid);
    if ours && flock(&fd, FlockOperation::NonBlockingLockExclusive).is_ok() {
        let _ignored = fs::remove_dir_all(path);
    }
}

// Worker pools and detached threads still own their output when the process exits, and a
// killed process never drops it, so earlier runs leave directories behind.
pub(super) fn prune_orphaned_private_outputs(tmp: &Path) {
    let Ok(entries) = fs::read_dir(tmp) else {
        return;
    };
    let uid = rustix::process::geteuid().as_raw();
    for entry in entries.flatten() {
        if is_private_output_name(&entry.file_name()) {
            remove_if_orphaned(&entry.path(), uid);
        }
    }
}
