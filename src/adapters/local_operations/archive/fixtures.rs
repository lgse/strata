// SPDX-License-Identifier: MIT

use super::{
    compression::{compress_7z, compress_tar, compress_zip},
    decoders::extract_zip_from_archive,
    extraction::ArchiveOutcome,
};
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::ArchiveFormat,
};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{Cursor, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

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
        is_hidden: false,
        mode: MetadataValue::Unknown,
    }
}

pub(super) fn compression_stages(destination: &Path) -> Result<Vec<OsString>, Box<dyn Error>> {
    Ok(fs::read_dir(destination)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().starts_with(".strata-compression-"))
        .collect())
}

pub(super) fn compression_stage_mode(destination: &Path) -> Result<u32, Box<dyn Error>> {
    let mut stages = compression_stages(destination)?;
    let name = stages.pop().ok_or("no compression staging file")?;
    if !stages.is_empty() {
        return Err("expected a single compression staging file".into());
    }
    Ok(fs::metadata(destination.join(name))?.permissions().mode() & 0o777)
}

pub(super) fn write_compression_fixture(
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
        ArchiveFormat::Tar => compress_tar(file, entries, false, &progress, &cancelled),
        ArchiveFormat::TarGz => compress_tar(file, entries, true, &progress, &cancelled),
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
    header.set_mode(0o644);
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

pub(super) fn never_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

pub(super) fn always_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

pub(super) fn completed_extract<T>(outcome: ArchiveOutcome<T>) -> Result<T, String> {
    match outcome {
        ArchiveOutcome::Completed(value) => Ok(value),
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

pub(super) const RAR_VERSION_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/fixtures/rar/version.rar");
pub(super) const RAR_ENCRYPTED_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/fixtures/rar/encrypted.rar");
pub(super) const RAR_COMMENT_HPW_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/fixtures/rar/comment-hpw-password.rar");
pub(super) const RAR_UNICODE_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/fixtures/rar/unicode.rar");
