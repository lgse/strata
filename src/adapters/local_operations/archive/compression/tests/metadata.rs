// SPDX-License-Identifier: MIT

//! The modes and times each format records for compressed sources.

use super::contents::{insert_once, tar_reader};
use super::*;

const SOURCE_TIME: i64 = 1_000_000_000;

#[derive(PartialEq, Eq)]
struct CompressedMetadata {
    mode: Option<u32>,
    modified: Option<i64>,
}

impl std::fmt::Debug for CompressedMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.mode {
            Some(mode) => write!(formatter, "mode {mode:o}, ")?,
            None => formatter.write_str("no mode, ")?,
        }
        match self.modified {
            Some(seconds) => write!(formatter, "modified {seconds}"),
            None => formatter.write_str("no time"),
        }
    }
}

type Metadata = BTreeMap<PathBuf, CompressedMetadata>;

fn zip_dos_seconds(time: zip::DateTime) -> Result<i64, Box<dyn Error>> {
    Ok(glib::DateTime::from_local(
        i32::from(time.year()),
        i32::from(time.month()),
        i32::from(time.day()),
        i32::from(time.hour()),
        i32::from(time.minute()),
        f64::from(time.second()),
    )?
    .to_unix())
}

fn zip_extended_time(entry: &zip::read::ZipFile<'_, fs::File>) -> Option<i64> {
    entry.extra_data_fields().find_map(|field| match field {
        zip::ExtraField::ExtendedTimestamp(stamp) => stamp.mod_time().map(i64::from),
        _ => None,
    })
}

fn read_zip_metadata(file: fs::File, password: Option<&str>) -> Result<Metadata, Box<dyn Error>> {
    let mut result = BTreeMap::new();
    let mut archive = zip::ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let options = zip::read::ZipReadOptions::new().password(password.map(str::as_bytes));
        let entry = archive.by_index_with_options(index, options)?;
        let modified = match zip_extended_time(&entry) {
            Some(seconds) => Some(seconds),
            None => entry.last_modified().map(zip_dos_seconds).transpose()?,
        };
        let metadata = CompressedMetadata {
            mode: entry.unix_mode().map(|mode| mode & 0o7777),
            modified,
        };
        let name = PathBuf::from(entry.name().expect("member name").trim_end_matches('/'));
        insert_once(&mut result, name, metadata);
    }
    Ok(result)
}

fn read_tar_metadata(file: fs::File, format: ArchiveFormat) -> Result<Metadata, Box<dyn Error>> {
    let mut result = BTreeMap::new();
    for entry in tar::Archive::new(tar_reader(file, format)).entries()? {
        let entry = entry?;
        let metadata = CompressedMetadata {
            mode: Some(entry.header().mode()? & 0o7777),
            modified: Some(i64::try_from(entry.header().mtime()?)?),
        };
        insert_once(&mut result, entry.path()?.into_owned(), metadata);
    }
    Ok(result)
}

fn sevenz_mode(entry: &sevenz_rust2::ArchiveEntry) -> Option<u32> {
    (entry.has_windows_attributes && entry.windows_attributes & 0x8000 != 0)
        .then_some((entry.windows_attributes >> 16) & 0o7777)
}

fn sevenz_modified(entry: &sevenz_rust2::ArchiveEntry) -> Result<Option<i64>, Box<dyn Error>> {
    if !entry.has_last_modified_date {
        return Ok(None);
    }
    let time = std::time::SystemTime::from(entry.last_modified_date);
    Ok(Some(i64::try_from(
        time.duration_since(std::time::UNIX_EPOCH)?.as_secs(),
    )?))
}

fn read_sevenz_metadata(
    file: fs::File,
    password: Option<&str>,
) -> Result<Metadata, Box<dyn Error>> {
    let mut result = BTreeMap::new();
    let archive = sevenz_rust2::ArchiveReader::new(
        file,
        password
            .map(sevenz_rust2::Password::from)
            .unwrap_or_default(),
    )?;
    for entry in &archive.archive().files {
        let metadata = CompressedMetadata {
            mode: sevenz_mode(entry),
            modified: sevenz_modified(entry)?,
        };
        insert_once(&mut result, PathBuf::from(entry.name()), metadata);
    }
    Ok(result)
}

fn read_compressed_metadata(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<Metadata, Box<dyn Error>> {
    let file = fs::File::open(path)?;
    match format {
        ArchiveFormat::Zip => read_zip_metadata(file, password),
        ArchiveFormat::Tar | ArchiveFormat::TarGz => read_tar_metadata(file, format),
        ArchiveFormat::SevenZ => read_sevenz_metadata(file, password),
        ArchiveFormat::Rar => Err("RAR compression is not supported".into()),
    }
}

/// A `dir` holding `run.sh`, `secret.txt` and, when asked, a `link`, all at `SOURCE_TIME`.
fn write_timed_source(root: &Path, with_link: bool) -> Result<PathBuf, Box<dyn Error>> {
    let source = root.join("dir");
    fs::create_dir(&source)?;
    fs::write(source.join("run.sh"), b"#!/bin/sh\n")?;
    fs::write(source.join("secret.txt"), b"secret")?;
    fs::set_permissions(source.join("run.sh"), fs::Permissions::from_mode(0o755))?;
    fs::set_permissions(source.join("secret.txt"), fs::Permissions::from_mode(0o600))?;
    if with_link {
        std::os::unix::fs::symlink("run.sh", source.join("link"))?;
        set_times_without_following(&source.join("link"), SOURCE_TIME)?;
    }
    set_times_without_following(&source.join("run.sh"), SOURCE_TIME)?;
    set_times_without_following(&source.join("secret.txt"), SOURCE_TIME)?;
    fs::set_permissions(&source, fs::Permissions::from_mode(0o750))?;
    set_times_without_following(&source, SOURCE_TIME)?;
    Ok(source)
}

fn expected_metadata(with_link: bool) -> Metadata {
    let mut expected = BTreeMap::from([
        (PathBuf::from("dir"), (0o750, SOURCE_TIME)),
        (PathBuf::from("dir/run.sh"), (0o755, SOURCE_TIME)),
        (PathBuf::from("dir/secret.txt"), (0o600, SOURCE_TIME)),
    ]);
    if with_link {
        expected.insert(PathBuf::from("dir/link"), (0o777, SOURCE_TIME));
    }
    expected
        .into_iter()
        .map(|(name, (mode, modified))| {
            (
                name,
                CompressedMetadata {
                    mode: Some(mode),
                    modified: Some(modified),
                },
            )
        })
        .collect()
}

type TimeParts = (i32, i32, i32, i32, i32, i32);

fn dos_time_parts(dos: zip::DateTime) -> TimeParts {
    (
        i32::from(dos.year()),
        i32::from(dos.month()),
        i32::from(dos.day()),
        i32::from(dos.hour()),
        i32::from(dos.minute()),
        i32::from(dos.second()),
    )
}

fn local_time_parts(local: &glib::DateTime) -> TimeParts {
    (
        local.year(),
        local.month(),
        local.day_of_month(),
        local.hour(),
        local.minute(),
        local.second(),
    )
}

/// Checks every ZIP member holds `SOURCE_TIME` as its local DOS time and its UT field.
fn expect_zip_times(archive: &Path, context: &str) -> Result<(), Box<dyn Error>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    let local = glib::DateTime::from_unix_local(SOURCE_TIME)?;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index)?;
        let dos = entry.last_modified().ok_or("ZIP entry has no DOS time")?;
        assert_eq!(
            dos_time_parts(dos),
            local_time_parts(&local),
            "{context}: DOS time of `{}`",
            entry.name().expect("member name")
        );
        assert_eq!(
            zip_extended_time(&entry),
            Some(SOURCE_TIME),
            "{context}: UT field of `{}`",
            entry.name().expect("member name")
        );
    }
    Ok(())
}

#[test]
fn compression_records_source_times_and_modes_in_every_format() -> Result<(), Box<dyn Error>> {
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
        (ArchiveFormat::SevenZ, None),
    ] {
        let context = format!("{format:?} password={}", password.is_some());
        let root = tempfile::tempdir()?;
        // 7z refuses links (`compression_reports_unsupported_7z_links_without_committing`).
        let with_link = format != ArchiveFormat::SevenZ;
        let source = write_timed_source(root.path(), with_link)?;
        let archive = root.path().join("archive");

        write_compression_fixture(&archive, &[source], format, password)?;

        assert_eq!(
            read_compressed_metadata(&archive, format, password)?,
            expected_metadata(with_link),
            "{context}"
        );
        if format == ArchiveFormat::Zip {
            expect_zip_times(&archive, &context)?;
        }
    }
    Ok(())
}

#[test]
fn zip_times_outside_the_dos_range_are_clamped_without_failing() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir(&source)?;
    let epoch = source.join("epoch.txt");
    let beyond_i32 = source.join("beyond-i32.txt");
    let beyond_dos = source.join("beyond-dos.txt");
    let beyond_i32_time = i64::from(u32::MAX) + 10;
    let beyond_dos_time = 4_360_000_000;
    for (path, seconds) in [
        (&epoch, 0),
        (&beyond_i32, beyond_i32_time),
        (&beyond_dos, beyond_dos_time),
    ] {
        fs::write(path, b"time")?;
        set_times_without_following(path, seconds)?;
    }
    let archive = root.path().join("archive.zip");

    write_compression_fixture(
        &archive,
        &[epoch, beyond_i32, beyond_dos],
        ArchiveFormat::Zip,
        None,
    )?;

    let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    let dos_parts = |time: zip::DateTime| {
        (
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second(),
        )
    };
    let entry = zip.by_name("epoch.txt")?;
    assert_eq!(
        entry.last_modified().map(dos_parts),
        Some(dos_parts(zip::DateTime::DEFAULT))
    );
    assert_eq!(zip_extended_time(&entry), Some(0));
    drop(entry);
    let entry = zip.by_name("beyond-i32.txt")?;
    let local = glib::DateTime::from_unix_local(beyond_i32_time)?;
    assert_eq!(
        entry.last_modified().map(dos_parts),
        Some((
            u16::try_from(local.year())?,
            u8::try_from(local.month())?,
            u8::try_from(local.day_of_month())?,
            u8::try_from(local.hour())?,
            u8::try_from(local.minute())?,
            u8::try_from(local.second() & !1)?,
        ))
    );
    assert_eq!(zip_extended_time(&entry), None);
    drop(entry);
    let entry = zip.by_name("beyond-dos.txt")?;
    assert_eq!(
        entry.last_modified().map(dos_parts),
        Some((2107, 12, 31, 23, 59, 58))
    );
    assert_eq!(zip_extended_time(&entry), None);
    Ok(())
}
