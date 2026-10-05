// SPDX-License-Identifier: MIT

use super::{
    compression::{compress_7z, compress_tar, compress_zip, inspect_archive_sources},
    decoders::extract_zip_from_archive,
    extraction::ArchiveOutcome,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::ArchiveFormat,
};
use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    io::{Cursor, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// GIO cannot trash test fixtures across devices into the home Trash.
pub(super) fn tempdir_on_home_device() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let home = gtk::glib::home_dir();
    let cache = gtk::glib::user_cache_dir();
    let parent = if same_device(&cache, &home) {
        cache
    } else {
        home
    };
    fs::create_dir_all(&parent)?;
    Ok(tempfile::Builder::new()
        .prefix(".strata-archive-test-")
        .tempdir_in(parent)?)
}

fn same_device(left: &Path, right: &Path) -> bool {
    fs::metadata(left)
        .and_then(|left| fs::metadata(right).map(|right| left.dev() == right.dev()))
        .unwrap_or(false)
}

/// Remove the fixture's Trash copy so host tests do not leave test files behind.
pub(super) struct HomeTrashGuard(PathBuf);

impl HomeTrashGuard {
    pub(super) fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
}

impl Drop for HomeTrashGuard {
    fn drop(&mut self) {
        let info_dir = gtk::glib::user_data_dir().join("Trash/info");
        let files_dir = gtk::glib::user_data_dir().join("Trash/files");
        let Ok(entries) = fs::read_dir(&info_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let info_path = entry.path();
            if info_path
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("trashinfo")
            {
                continue;
            }
            let Ok(text) = fs::read_to_string(&info_path) else {
                continue;
            };
            let Some(encoded) = text.lines().find_map(|line| line.strip_prefix("Path=")) else {
                continue;
            };
            if crate::adapters::trash_restore::decode_trashinfo_path(encoded).as_deref()
                != Some(self.0.as_path())
            {
                continue;
            }
            if let Some(name) = info_path.file_stem() {
                let _removed = fs::remove_file(files_dir.join(name));
            }
            let _removed = fs::remove_file(info_path);
        }
    }
}

pub(super) fn test_file_entry(path: &Path) -> FileEntry {
    let name = path.file_name().unwrap_or_default().to_os_string();
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.clone(),
        display_name: name.to_string_lossy().into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    }
}

pub(super) const COMPRESSION_STAGE: &str = ".strata-compression-";
pub(super) const EXTRACTION_STAGE: &str = ".strata-extraction-";

pub(super) fn stages(destination: &Path, prefix: &str) -> Result<Vec<OsString>, Box<dyn Error>> {
    Ok(fs::read_dir(destination)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().starts_with(prefix))
        .collect())
}

pub(super) fn compression_stage_mode(destination: &Path) -> Result<u32, Box<dyn Error>> {
    let mut found = stages(destination, COMPRESSION_STAGE)?;
    let name = found.pop().ok_or("no compression staging file")?;
    if !found.is_empty() {
        return Err("expected a single compression staging file".into());
    }
    Ok(fs::metadata(destination.join(name))?.permissions().mode() & 0o777)
}

pub(crate) fn write_compression_fixture(
    path: &Path,
    entries: &[PathBuf],
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<usize, String> {
    let file = fs::File::create(path).map_err(|error| error.to_string())?;
    let progress = Arc::new(AtomicUsize::new(0));
    let cancelled = never_cancelled();
    match format {
        ArchiveFormat::Zip => compress_zip(file, entries, password, &progress, &cancelled),
        ArchiveFormat::SevenZ => compress_7z(file, entries, password, &progress, &cancelled),
        ArchiveFormat::Tar => compress_tar(file, entries, None, &progress, &cancelled),
        ArchiveFormat::TarGz => compress_tar(
            file,
            entries,
            Some(
                inspect_archive_sources(entries, &cancelled)
                    .map_err(|error| error.to_string())?
                    .gzip_level(),
            ),
            &progress,
            &cancelled,
        ),
        ArchiveFormat::Rar => return Err("RAR compression is not supported".to_owned()),
    }
    .map_err(|error| error.to_string())?;
    Ok(progress.load(Ordering::Relaxed))
}

pub(super) fn write_zip(path: &Path, entries: &[(&str, &[u8])]) -> Result<(), Box<dyn Error>> {
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    for (name, contents) in entries {
        writer.start_file(*name, zip::write::SimpleFileOptions::default())?;
        writer.write_all(contents)?;
    }
    writer.finish()?;
    Ok(())
}

pub(super) fn patch_zip_uncompressed_size(
    path: &Path,
    uncompressed_size: u32,
) -> Result<(), Box<dyn Error>> {
    const CENTRAL_DIRECTORY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const UNCOMPRESSED_SIZE_OFFSET: usize = 24;
    let mut bytes = fs::read(path)?;
    let record = bytes
        .windows(CENTRAL_DIRECTORY_SIGNATURE.len())
        .position(|window| window == CENTRAL_DIRECTORY_SIGNATURE)
        .ok_or("zip fixture has no central directory record")?;
    let field = record + UNCOMPRESSED_SIZE_OFFSET;
    bytes[field..field + 4].copy_from_slice(&uncompressed_size.to_le_bytes());
    fs::write(path, bytes)?;
    Ok(())
}

fn append_raw_tar_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    entry_type: tar::EntryType,
    name: &str,
    contents: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut header = tar::Header::new_gnu();
    header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
    header.set_mode(if entry_type.is_dir() { 0o755 } else { 0o644 });
    header.set_size(contents.len() as u64);
    header.set_entry_type(entry_type);
    header.set_cksum();
    builder.append(&header, contents)?;
    Ok(())
}

pub(super) fn write_tar(
    path: &Path,
    name: &str,
    contents: &[u8],
    gzip: bool,
) -> Result<(), Box<dyn Error>> {
    write_tar_entries(path, &[(tar::EntryType::Regular, name, contents)], gzip)
}

pub(super) fn write_tar_entries(
    path: &Path,
    entries: &[(tar::EntryType, &str, &[u8])],
    gzip: bool,
) -> Result<(), Box<dyn Error>> {
    let file = fs::File::create(path)?;
    if gzip {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        for (entry_type, name, contents) in entries {
            append_raw_tar_entry(&mut builder, *entry_type, name, contents)?;
        }
        builder.into_inner()?.finish()?;
    } else {
        let mut builder = tar::Builder::new(file);
        for (entry_type, name, contents) in entries {
            append_raw_tar_entry(&mut builder, *entry_type, name, contents)?;
        }
        builder.finish()?;
    }
    Ok(())
}

pub(super) fn split_into_gzip_members(path: &Path, split_at: usize) -> Result<(), Box<dyn Error>> {
    let tar = fs::read(path)?;
    let mut members = Vec::new();
    for part in [&tar[..split_at], &tar[split_at..]] {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(part)?;
        members.extend(encoder.finish()?);
    }
    fs::write(path, members)?;
    Ok(())
}

/// Trailer offsets: 8 is CRC32, 4 is ISIZE.
pub(super) fn corrupt_gzip_trailer(
    path: &Path,
    offset_from_end: usize,
) -> Result<(), Box<dyn Error>> {
    let mut bytes = fs::read(path)?;
    let index = bytes
        .len()
        .checked_sub(offset_from_end)
        .ok_or("gzip fixture is shorter than its trailer")?;
    bytes[index] ^= 0xff;
    fs::write(path, bytes)?;
    Ok(())
}

pub(super) fn write_7z(path: &Path, name: &str, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    write_7z_entries(path, &[(name, contents)])
}

pub(super) fn write_7z_entries(
    path: &Path,
    entries: &[(&str, &[u8])],
) -> Result<(), Box<dyn Error>> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path)?;
    for (name, contents) in entries {
        writer.push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file(name),
            Some(Cursor::new(*contents)),
        )?;
    }
    writer.finish()?;
    Ok(())
}

pub(super) fn write_7z_stored(
    path: &Path,
    entries: &[(&str, &[u8])],
) -> Result<(), Box<dyn Error>> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path)?;
    writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    for (name, contents) in entries {
        writer.push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file(name),
            Some(Cursor::new(*contents)),
        )?;
    }
    writer.finish()?;
    Ok(())
}

pub(super) fn patch_zip_entry_count(path: &Path, count: u16) -> Result<(), Box<dyn Error>> {
    const END_OF_CENTRAL_DIRECTORY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const DISK_ENTRY_COUNT_OFFSET: usize = 8;
    const TOTAL_ENTRY_COUNT_OFFSET: usize = 10;
    let mut bytes = fs::read(path)?;
    let record = bytes
        .windows(END_OF_CENTRAL_DIRECTORY_SIGNATURE.len())
        .rposition(|window| window == END_OF_CENTRAL_DIRECTORY_SIGNATURE)
        .ok_or("zip fixture has no end-of-central-directory record")?;
    bytes[record + DISK_ENTRY_COUNT_OFFSET..record + DISK_ENTRY_COUNT_OFFSET + 2]
        .copy_from_slice(&count.to_le_bytes());
    bytes[record + TOTAL_ENTRY_COUNT_OFFSET..record + TOTAL_ENTRY_COUNT_OFFSET + 2]
        .copy_from_slice(&count.to_le_bytes());
    fs::write(path, bytes)?;
    Ok(())
}

pub(super) fn never_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

pub(super) fn always_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

pub(super) fn completed_extract(
    outcome: ArchiveOutcome<Option<String>>,
) -> Result<Option<String>, String> {
    match outcome {
        ArchiveOutcome::Completed(first_name) => Ok(first_name),
        ArchiveOutcome::Cancelled { .. } => Err("unexpected cancellation".to_owned()),
    }
}

pub(super) fn extract_zip(path: &Path, destination: &Path) -> Result<Option<String>, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    completed_extract(
        extract_zip_from_archive(
            &mut archive,
            destination,
            path.file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("archive.zip"),
            None,
            &Arc::new(AtomicUsize::new(0)),
            &never_cancelled(),
        )
        .map_err(|error| error.to_string())?,
    )
}

pub(super) fn write_zip_stored(
    path: &Path,
    entries: &[(&str, &[u8])],
) -> Result<(), Box<dyn Error>> {
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, contents) in entries {
        writer.start_file(*name, options)?;
        writer.write_all(contents)?;
    }
    writer.finish()?;
    Ok(())
}

pub(super) enum FixtureMember<'a> {
    File {
        name: &'a str,
        contents: &'a [u8],
        mode: Option<u32>,
        modified: Option<u64>,
    },
    Directory {
        name: &'a str,
        mode: Option<u32>,
        modified: Option<u64>,
    },
    Symlink {
        name: &'a str,
        target: &'a [u8],
    },
    HardLink {
        name: &'a str,
        target: &'a str,
    },
}

const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

/// Info-ZIP extended timestamp (`UT`, `0x5455`) carrying only the modification time.
pub(super) fn zip_extended_timestamp(seconds: u64) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut field = vec![1];
    field.extend_from_slice(&i32::try_from(seconds)?.to_le_bytes());
    Ok(field)
}

pub(super) fn write_members(
    path: &Path,
    format: ArchiveFormat,
    members: &[FixtureMember<'_>],
) -> Result<(), Box<dyn Error>> {
    match format {
        ArchiveFormat::Zip => write_zip_members(path, members),
        ArchiveFormat::Tar => write_tar_members(path, members, false),
        ArchiveFormat::TarGz => write_tar_members(path, members, true),
        ArchiveFormat::SevenZ => write_7z_members(path, members),
        ArchiveFormat::Rar => Err("RAR fixtures cannot be written".into()),
    }
}

fn write_zip_members(path: &Path, members: &[FixtureMember<'_>]) -> Result<(), Box<dyn Error>> {
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    for member in members {
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::DEFAULT)
            .into_full_options();
        match member {
            FixtureMember::File {
                name,
                contents,
                mode,
                modified,
            } => {
                if let Some(mode) = mode {
                    options = options.unix_permissions(*mode);
                }
                if let Some(seconds) = modified {
                    options.add_extra_data(0x5455, zip_extended_timestamp(*seconds)?, false)?;
                }
                writer.start_file(*name, options)?;
                writer.write_all(contents)?;
            }
            FixtureMember::Directory {
                name,
                mode,
                modified,
            } => {
                if let Some(mode) = mode {
                    options = options.unix_permissions(*mode);
                }
                if let Some(seconds) = modified {
                    options.add_extra_data(0x5455, zip_extended_timestamp(*seconds)?, false)?;
                }
                writer.add_directory(*name, options)?;
            }
            FixtureMember::Symlink { name, target } => {
                writer.add_symlink(*name, std::str::from_utf8(target)?, options)?;
            }
            FixtureMember::HardLink { .. } => return Err("ZIP fixtures have no hard links".into()),
        }
    }
    writer.finish()?;
    Ok(())
}

fn tar_member_header(member: &FixtureMember<'_>) -> Result<tar::Header, Box<dyn Error>> {
    let mut header = tar::Header::new_gnu();
    let (name, entry_type, mode, modified) = match member {
        FixtureMember::File {
            name,
            mode,
            modified,
            ..
        } => (
            *name,
            tar::EntryType::Regular,
            mode.unwrap_or(0o644),
            *modified,
        ),
        FixtureMember::Directory {
            name,
            mode,
            modified,
        } => (
            *name,
            tar::EntryType::Directory,
            mode.unwrap_or(0o755),
            *modified,
        ),
        FixtureMember::Symlink { name, target } => {
            header.set_link_name_literal(target)?;
            (*name, tar::EntryType::Symlink, 0o777, None)
        }
        FixtureMember::HardLink { name, target } => {
            header.set_link_name_literal(target.as_bytes())?;
            (*name, tar::EntryType::Link, 0o644, None)
        }
    };
    header.set_path(name)?;
    header.set_entry_type(entry_type);
    header.set_mode(mode);
    header.set_mtime(modified.unwrap_or(0));
    Ok(header)
}

fn append_tar_members<W: Write>(
    builder: &mut tar::Builder<W>,
    members: &[FixtureMember<'_>],
) -> Result<(), Box<dyn Error>> {
    for member in members {
        let mut header = tar_member_header(member)?;
        let contents: &[u8] = match member {
            FixtureMember::File { contents, .. } => contents,
            _ => &[],
        };
        header.set_size(contents.len() as u64);
        header.set_cksum();
        builder.append(&header, contents)?;
    }
    Ok(())
}

fn write_tar_members(
    path: &Path,
    members: &[FixtureMember<'_>],
    gzip: bool,
) -> Result<(), Box<dyn Error>> {
    let file = fs::File::create(path)?;
    if gzip {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        append_tar_members(&mut builder, members)?;
        builder.into_inner()?.finish()?;
    } else {
        let mut builder = tar::Builder::new(file);
        append_tar_members(&mut builder, members)?;
        builder.finish()?;
    }
    Ok(())
}

fn seven_z_attributes(entry: &mut sevenz_rust2::ArchiveEntry, st_mode: u32) {
    entry.has_windows_attributes = true;
    entry.windows_attributes = 0x8000 | (st_mode << 16);
}

fn seven_z_modified(
    entry: &mut sevenz_rust2::ArchiveEntry,
    modified: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    if let Some(seconds) = modified {
        entry.has_last_modified_date = true;
        entry.last_modified_date = sevenz_rust2::NtTime::try_from(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
        )
        .map_err(|error| format!("{error:?}"))?;
    } else {
        entry.has_last_modified_date = false;
    }
    Ok(())
}

fn write_7z_members(path: &Path, members: &[FixtureMember<'_>]) -> Result<(), Box<dyn Error>> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path)?;
    writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    for member in members {
        match member {
            FixtureMember::File {
                name,
                contents,
                mode,
                modified,
            } => {
                let mut entry = sevenz_rust2::ArchiveEntry::new_file(name);
                if let Some(mode) = mode {
                    seven_z_attributes(&mut entry, S_IFREG | mode);
                }
                seven_z_modified(&mut entry, *modified)?;
                writer.push_archive_entry(entry, Some(Cursor::new(*contents)))?;
            }
            FixtureMember::Directory {
                name,
                mode,
                modified,
            } => {
                let mut entry = sevenz_rust2::ArchiveEntry::new_directory(name);
                if let Some(mode) = mode {
                    seven_z_attributes(&mut entry, S_IFDIR | mode);
                }
                seven_z_modified(&mut entry, *modified)?;
                writer.push_archive_entry::<&[u8]>(entry, None)?;
            }
            FixtureMember::Symlink { name, target } => {
                let mut entry = sevenz_rust2::ArchiveEntry::new_file(name);
                seven_z_attributes(&mut entry, S_IFLNK | 0o777);
                seven_z_modified(&mut entry, None)?;
                writer.push_archive_entry(entry, Some(Cursor::new(*target)))?;
            }
            FixtureMember::HardLink { .. } => return Err("7z fixtures have no hard links".into()),
        }
    }
    writer.finish()?;
    Ok(())
}

/// Overwrites the external attributes of every central-directory record;
/// `0` makes `ZipFile::unix_mode()` report no mode.
pub(super) fn patch_zip_external_attributes(path: &Path, value: u32) -> Result<(), Box<dyn Error>> {
    const CENTRAL_DIRECTORY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const EXTERNAL_ATTRIBUTES_OFFSET: usize = 38;
    let mut bytes = fs::read(path)?;
    let records = bytes
        .windows(CENTRAL_DIRECTORY_SIGNATURE.len())
        .enumerate()
        .filter(|(_, window)| *window == CENTRAL_DIRECTORY_SIGNATURE)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Err("zip fixture has no central directory record".into());
    }
    for record in records {
        let field = record + EXTERNAL_ATTRIBUTES_OFFSET;
        bytes[field..field + 4].copy_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes)?;
    Ok(())
}

pub(super) fn expected_mode(mode: u32) -> u32 {
    mode & 0o777 & !super::destination::process_umask()
}

pub(super) fn set_times_without_following(path: &Path, seconds: i64) -> Result<(), Box<dyn Error>> {
    let time = rustix::fs::Timespec {
        tv_sec: seconds,
        tv_nsec: 0,
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        path,
        &rustix::fs::Timestamps {
            last_access: time,
            last_modification: time,
        },
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    )?;
    Ok(())
}
