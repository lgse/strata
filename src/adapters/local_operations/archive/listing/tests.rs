// SPDX-License-Identifier: MIT

use super::{
    ARCHIVE_TOO_LARGE_MESSAGE, ArchiveListingStatus, BudgetReader, MAX_ARCHIVE_ENTRIES,
    MAX_TAR_GZ_DECOMPRESSED_BYTES, collect_tar_seekable, decode_archive_listing,
    encode_archive_result, ensure_entry_budget, list_archive_entries_direct,
};
use crate::{
    adapters::local_operations::archive::fixtures::{
        always_cancelled, never_cancelled, patch_zip_entry_count, write_7z, write_7z_entries,
        write_7z_stored, write_compression_fixture, write_tar, write_tar_entries, write_zip_stored,
    },
    services::ArchiveFormat,
};

fn list_entries(path: &std::path::Path, format: ArchiveFormat) -> Vec<(String, bool, u64)> {
    list_archive_entries_direct(path, format, None, &never_cancelled())
        .expect("listing should succeed")
        .entries
        .into_iter()
        .map(|entry| (entry.name, entry.directory, entry.size))
        .collect()
}

fn listing_status(
    path: &std::path::Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> ArchiveListingStatus {
    list_archive_entries_direct(path, format, password, &never_cancelled())
        .expect("listing should succeed")
        .status
}

#[test]
fn zip_listing_reads_members_headers_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.zip");
    write_zip_stored(
        &path,
        &[
            ("src/lib.rs", b"fn main(){}"),
            ("src/hello.txt", b"hi"),
            ("Cargo.toml", b"[package]"),
        ],
    )
    .expect("write zip");
    assert_eq!(
        list_entries(&path, ArchiveFormat::Zip),
        vec![
            ("src/lib.rs".to_owned(), false, 11),
            ("src/hello.txt".to_owned(), false, 2),
            ("Cargo.toml".to_owned(), false, 9),
        ]
    );
}

#[test]
fn zip_listing_rejects_duplicate_members_instead_of_hiding_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("duplicates.zip");
    write_zip_stored(&path, &[("first.txt", b"first"), ("other.txt", b"second")])
        .expect("write zip");
    let mut bytes = std::fs::read(&path).expect("read zip");
    let offsets: Vec<_> = bytes
        .windows(9)
        .enumerate()
        .filter_map(|(index, value)| (value == b"other.txt").then_some(index))
        .collect();
    assert_eq!(offsets.len(), 2);
    for offset in offsets {
        bytes[offset..offset + 9].copy_from_slice(b"first.txt");
    }
    std::fs::write(&path, bytes).expect("write duplicate names");
    match list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::ARCHIVE_UNSUPPORTED_MESSAGE),
        Ok(_) => panic!("duplicate ZIP members must not silently collapse"),
    }
}

#[test]
fn zip_listing_distinguishes_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("dirs.zip");
    write_zip_stored(&path, &[("folder/child.txt", b"x")]).expect("write zip");
    let entries = list_entries(&path, ArchiveFormat::Zip);
    assert!(entries.contains(&("folder/child.txt".to_owned(), false, 1)));
}

#[test]
fn tar_listing_reads_headers_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.tar");
    write_tar(&path, "hello.txt", b"hi", false).expect("write tar");
    assert_eq!(
        list_entries(&path, ArchiveFormat::Tar),
        vec![("hello.txt".to_owned(), false, 2)]
    );
}

#[test]
fn tar_gz_listing_decompresses_headers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.tar.gz");
    write_tar(&path, "hello.txt", b"hi", true).expect("write tar.gz");
    assert_eq!(
        list_entries(&path, ArchiveFormat::TarGz),
        vec![("hello.txt".to_owned(), false, 2)]
    );
}

#[test]
fn tar_listing_drops_extended_headers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("extended.tar");
    write_tar_entries(
        &path,
        &[
            (tar::EntryType::XGlobalHeader, "/pax-global-header", b""),
            (tar::EntryType::Regular, "real.txt", b"data"),
        ],
        false,
    )
    .expect("write tar");
    assert_eq!(
        list_entries(&path, ArchiveFormat::Tar),
        vec![("real.txt".to_owned(), false, 4)]
    );
}

#[test]
fn sevenz_listing_reads_members_headers_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.7z");
    write_7z(&path, "hello.txt", b"hi").expect("write 7z");
    assert_eq!(
        list_entries(&path, ArchiveFormat::SevenZ),
        vec![("hello.txt".to_owned(), false, 2)]
    );
}

#[test]
fn sevenz_listing_distinguishes_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("dirs.7z");
    write_7z_entries(&path, &[("folder/child.txt", b"x")]).expect("write 7z");
    assert!(list_entries(&path, ArchiveFormat::SevenZ).contains(&(
        "folder/child.txt".to_owned(),
        false,
        1
    )));
}

#[test]
fn corrupt_zip_listing_fails_cleanly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("broken.zip");
    std::fs::write(&path, b"not a zip archive at all").expect("write junk");
    let result = list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled());
    assert!(result.is_err());
}

#[test]
fn listing_cancels_between_members() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.zip");
    write_zip_stored(&path, &[("a.txt", b"a"), ("b.txt", b"b")]).expect("write zip");
    assert!(
        list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &always_cancelled()).is_err(),
        "a pre-cancelled listing should abort"
    );
}

#[test]
fn names_are_preserved_verbatim() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("uni.zip");
    write_zip_stored(&path, &[("héllo wörld/文件.txt", b"x")]).expect("write zip");
    assert_eq!(
        list_entries(&path, ArchiveFormat::Zip),
        vec![("héllo wörld/文件.txt".to_owned(), false, 1,)]
    );
}

fn encrypted_fixture(
    dir: &tempfile::TempDir,
    format: ArchiveFormat,
    name: &str,
    password: Option<&str>,
) -> std::path::PathBuf {
    let source = dir.path().join("folder");
    std::fs::create_dir_all(source.join("nested")).expect("create source dir");
    std::fs::write(source.join("item.txt"), b"contents").expect("write source file");
    let archive = dir.path().join(name);
    write_compression_fixture(&archive, std::slice::from_ref(&source), format, password)
        .expect("write fixture");
    archive
}

#[test]
fn encrypted_zip_lists_names_and_asks_for_password() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = encrypted_fixture(&dir, ArchiveFormat::Zip, "secret.zip", Some("s3cret"));
    assert_eq!(
        listing_status(&path, ArchiveFormat::Zip, None),
        ArchiveListingStatus::NeedsPassword
    );
    let entries = list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled())
        .expect("listing should succeed")
        .entries;
    assert!(entries.iter().any(|entry| entry.name == "folder/item.txt"));
}

#[test]
fn encrypted_zip_verifies_the_supplied_password() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = encrypted_fixture(&dir, ArchiveFormat::Zip, "secret.zip", Some("s3cret"));
    assert_eq!(
        listing_status(&path, ArchiveFormat::Zip, Some("s3cret")),
        ArchiveListingStatus::Open
    );
    assert_eq!(
        listing_status(&path, ArchiveFormat::Zip, Some("wrong")),
        ArchiveListingStatus::WrongPassword
    );
    assert_eq!(
        listing_status(&path, ArchiveFormat::Zip, Some("")),
        ArchiveListingStatus::WrongPassword
    );
}

#[test]
fn encrypted_sevenz_lists_only_after_unlocking_the_header() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = encrypted_fixture(&dir, ArchiveFormat::SevenZ, "secret.7z", Some("s3cret"));
    assert_eq!(
        listing_status(&path, ArchiveFormat::SevenZ, None),
        ArchiveListingStatus::NeedsPassword
    );
    assert_eq!(
        listing_status(&path, ArchiveFormat::SevenZ, Some("s3cret")),
        ArchiveListingStatus::Open
    );
    assert_eq!(
        listing_status(&path, ArchiveFormat::SevenZ, Some("wrong")),
        ArchiveListingStatus::WrongPassword
    );
}

#[test]
fn plain_sevenz_with_encrypted_content_is_listed_without_a_password() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("content.7z");
    let mut writer = sevenz_rust2::ArchiveWriter::new(std::fs::File::create(&path).expect("file"))
        .expect("writer");
    writer.set_encrypt_header(false);
    writer.set_content_methods(vec![
        sevenz_rust2::encoder_options::AesEncoderOptions::new("s3cret".into()).into(),
        sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::LZMA2),
    ]);
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file("folder/item.txt"),
            Some(std::io::Cursor::new(b"contents")),
        )
        .expect("push entry");
    writer.finish().expect("finish");
    assert_eq!(
        listing_status(&path, ArchiveFormat::SevenZ, None),
        ArchiveListingStatus::Open
    );
}

#[test]
fn tar_and_tar_gz_listing_ignores_passwords() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tar = encrypted_fixture(&dir, ArchiveFormat::Tar, "plain.tar", None);
    assert_eq!(
        listing_status(&tar, ArchiveFormat::Tar, Some("bogus")),
        ArchiveListingStatus::Open
    );
    let gzip = encrypted_fixture(&dir, ArchiveFormat::TarGz, "plain.tar.gz", None);
    assert_eq!(
        listing_status(&gzip, ArchiveFormat::TarGz, Some("bogus")),
        ArchiveListingStatus::Open
    );
}

fn large_tar_fixture(dir: &tempfile::TempDir) -> (std::path::PathBuf, usize) {
    const PAYLOAD_LEN: usize = 4 * 1024 * 1024;
    let payload: Vec<u8> = (0..PAYLOAD_LEN).map(|index| (index % 251) as u8).collect();
    let path = dir.path().join("large.tar");
    write_tar_entries(
        &path,
        &[
            (tar::EntryType::Regular, "big.bin", &payload),
            (tar::EntryType::Regular, "small.txt", b"hi"),
        ],
        false,
    )
    .expect("write tar");
    (path, PAYLOAD_LEN)
}

#[test]
fn large_plain_tar_members_are_listed_with_correct_sizes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, payload_len) = large_tar_fixture(&dir);
    assert_eq!(
        list_entries(&path, ArchiveFormat::Tar),
        vec![
            ("big.bin".to_owned(), false, payload_len as u64),
            ("small.txt".to_owned(), false, 2),
        ]
    );
}

struct CountingReader<R> {
    inner: R,
    bytes_read: std::rc::Rc<std::cell::Cell<u64>>,
    seeks: std::rc::Rc<std::cell::Cell<u64>>,
}

impl<R: std::io::Read> std::io::Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.bytes_read.set(self.bytes_read.get() + read as u64);
        Ok(read)
    }
}

impl<R: std::io::Seek> std::io::Seek for CountingReader<R> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        let position = self.inner.seek(position)?;
        self.seeks.set(self.seeks.get() + 1);
        Ok(position)
    }
}

#[test]
fn plain_tar_listing_seeks_past_member_data_instead_of_reading_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, payload_len) = large_tar_fixture(&dir);
    let file_len = std::fs::metadata(&path).expect("fixture size").len();

    let bytes_read = std::rc::Rc::new(std::cell::Cell::new(0));
    let seeks = std::rc::Rc::new(std::cell::Cell::new(0));
    let reader = CountingReader {
        inner: std::fs::File::open(&path).expect("open fixture"),
        bytes_read: bytes_read.clone(),
        seeks: seeks.clone(),
    };
    let listing = collect_tar_seekable(reader, &never_cancelled()).expect("listing should succeed");
    assert_eq!(listing.status, ArchiveListingStatus::Open);
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.directory, entry.size))
            .collect::<Vec<_>>(),
        vec![
            ("big.bin".to_owned(), false, payload_len as u64),
            ("small.txt".to_owned(), false, 2),
        ]
    );
    assert!(
        seeks.get() >= 1,
        "expected at least one seek, got {}",
        seeks.get()
    );
    assert!(
        bytes_read.get() * 16 < file_len,
        "listing read {} of {file_len} bytes instead of seeking past member data",
        bytes_read.get()
    );
}

#[test]
fn tar_listing_cancels_between_members() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.tar");
    write_tar(&path, "a.txt", b"a", false).expect("write tar");
    assert!(
        list_archive_entries_direct(&path, ArchiveFormat::Tar, None, &always_cancelled()).is_err(),
        "a pre-cancelled listing should abort"
    );
}

#[test]
fn truncated_plain_tar_listing_reports_an_invalid_archive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("truncated.tar");
    write_tar(&path, "cut.bin", &vec![0xABu8; 1024 * 1024], false).expect("write tar");
    let bytes = std::fs::read(&path).expect("read");
    std::fs::write(&path, &bytes[..1024]).expect("truncate");
    match list_archive_entries_direct(&path, ArchiveFormat::Tar, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("truncated tar must not list successfully"),
    }
}

#[test]
fn entry_budget_accepts_up_to_the_cap_and_rejects_beyond_it() {
    assert!(ensure_entry_budget(0).is_ok());
    assert!(ensure_entry_budget(MAX_ARCHIVE_ENTRIES - 1).is_ok());
    assert_eq!(
        ensure_entry_budget(MAX_ARCHIVE_ENTRIES),
        Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned())
    );
}

fn many_names(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("f{index:05}.txt")).collect()
}

fn assert_too_large(result: Result<super::ArchiveListing, String>) {
    match result {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("over-limit listing must not succeed"),
    }
}

#[test]
fn zip_listing_rejects_more_than_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("many.zip");
    let names = many_names(MAX_ARCHIVE_ENTRIES + 1);
    write_zip_stored(
        &path,
        &names
            .iter()
            .map(|name| (name.as_str(), b"" as &[u8]))
            .collect::<Vec<_>>(),
    )
    .expect("write zip");
    assert_too_large(list_archive_entries_direct(
        &path,
        ArchiveFormat::Zip,
        None,
        &never_cancelled(),
    ));
}

#[test]
fn zip_listing_accepts_exactly_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("capped.zip");
    let names = many_names(MAX_ARCHIVE_ENTRIES);
    write_zip_stored(
        &path,
        &names
            .iter()
            .map(|name| (name.as_str(), b"" as &[u8]))
            .collect::<Vec<_>>(),
    )
    .expect("write zip");
    let listing = list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled())
        .expect("at-cap listing succeeds");
    assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES);
}

#[test]
fn inflated_zip_entry_count_fails_without_hanging() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("inflated.zip");
    write_zip_stored(&path, &[("real.txt", b"x")]).expect("write zip");
    patch_zip_entry_count(&path, u16::MAX).expect("patch count");
    assert!(
        list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled()).is_err(),
        "a central directory claiming 65535 entries must fail promptly"
    );
}

#[test]
fn sevenz_listing_rejects_more_than_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("many.7z");
    let names = many_names(MAX_ARCHIVE_ENTRIES + 1);
    write_7z_stored(
        &path,
        &names
            .iter()
            .map(|name| (name.as_str(), b"" as &[u8]))
            .collect::<Vec<_>>(),
    )
    .expect("write 7z");
    assert_too_large(list_archive_entries_direct(
        &path,
        ArchiveFormat::SevenZ,
        None,
        &never_cancelled(),
    ));
}

#[test]
fn sevenz_listing_accepts_exactly_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("capped.7z");
    let names = many_names(MAX_ARCHIVE_ENTRIES);
    write_7z_stored(
        &path,
        &names
            .iter()
            .map(|name| (name.as_str(), b"" as &[u8]))
            .collect::<Vec<_>>(),
    )
    .expect("write 7z");
    let listing =
        list_archive_entries_direct(&path, ArchiveFormat::SevenZ, None, &never_cancelled())
            .expect("at-cap listing succeeds");
    assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES);
}

fn tar_many_fixture(
    dir: &tempfile::TempDir,
    name: &str,
    count: usize,
    gzip: bool,
) -> std::path::PathBuf {
    let path = dir.path().join(name);
    let names = many_names(count);
    write_tar_entries(
        &path,
        &names
            .iter()
            .map(|name| (tar::EntryType::Regular, name.as_str(), b"" as &[u8]))
            .collect::<Vec<_>>(),
        gzip,
    )
    .expect("write tar");
    path
}

#[test]
fn tar_listing_rejects_more_than_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = tar_many_fixture(&dir, "many.tar", MAX_ARCHIVE_ENTRIES + 1, false);
    assert_too_large(list_archive_entries_direct(
        &path,
        ArchiveFormat::Tar,
        None,
        &never_cancelled(),
    ));
}

#[test]
fn tar_listing_accepts_exactly_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = tar_many_fixture(&dir, "capped.tar", MAX_ARCHIVE_ENTRIES, false);
    let listing = list_archive_entries_direct(&path, ArchiveFormat::Tar, None, &never_cancelled())
        .expect("at-cap listing succeeds");
    assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES);
}

#[test]
fn tar_gz_listing_rejects_more_than_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = tar_many_fixture(&dir, "many.tar.gz", MAX_ARCHIVE_ENTRIES + 1, true);
    assert_too_large(list_archive_entries_direct(
        &path,
        ArchiveFormat::TarGz,
        None,
        &never_cancelled(),
    ));
}

#[test]
fn tar_gz_listing_accepts_exactly_max_archive_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = tar_many_fixture(&dir, "capped.tar.gz", MAX_ARCHIVE_ENTRIES, true);
    let listing =
        list_archive_entries_direct(&path, ArchiveFormat::TarGz, None, &never_cancelled())
            .expect("at-cap listing succeeds");
    assert_eq!(listing.entries.len(), MAX_ARCHIVE_ENTRIES);
}

#[test]
fn tar_gz_listing_rejects_oversized_compressed_inputs_before_decoding() {
    use super::MAX_TAR_GZ_COMPRESSED_BYTES;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("huge.tar.gz");
    std::fs::File::create(&path)
        .expect("create sparse input")
        .set_len(MAX_TAR_GZ_COMPRESSED_BYTES + 1)
        .expect("size sparse input");
    match list_archive_entries_direct(&path, ArchiveFormat::TarGz, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("oversized compressed input must not list"),
    }
}

#[test]
fn tar_gz_listing_accepts_large_members_within_the_decompressed_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("large.tar.gz");
    let payload: Vec<u8> = (0..4 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    write_tar_entries(
        &path,
        &[
            (tar::EntryType::Regular, "big.bin", &payload),
            (tar::EntryType::Regular, "small.txt", b"hi"),
        ],
        true,
    )
    .expect("write tar.gz");
    assert_eq!(
        list_entries(&path, ArchiveFormat::TarGz),
        vec![
            ("big.bin".to_owned(), false, payload.len() as u64),
            ("small.txt".to_owned(), false, 2),
        ]
    );
}

fn budget_reader<'a>(
    data: &'a [u8],
    budget: u64,
    cancelled: &'a std::sync::atomic::AtomicBool,
) -> BudgetReader<'a, &'a [u8]> {
    BudgetReader {
        inner: data,
        remaining: budget,
        cancelled,
    }
}

#[test]
fn budget_reader_passes_through_reads_within_budget() {
    use std::io::Read;

    let flag = std::sync::atomic::AtomicBool::new(false);
    let mut reader = budget_reader(b"hello world", 11, &flag);
    let mut out = Vec::new();
    reader.read_to_end(&mut out).expect("read");
    assert_eq!(out, b"hello world");
}

#[test]
fn budget_reader_reports_clean_eof_at_exact_budget() {
    use std::io::Read;

    let flag = std::sync::atomic::AtomicBool::new(false);
    let mut reader = budget_reader(b"hello", 5, &flag);
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .expect("exact budget is not an error");
    assert_eq!(out, b"hello");
}

#[test]
fn budget_reader_fails_once_reads_would_exceed_budget() {
    use std::io::Read;

    let flag = std::sync::atomic::AtomicBool::new(false);
    let mut reader = budget_reader(b"hello world", 5, &flag);
    let mut out = Vec::new();
    let error = reader
        .read_to_end(&mut out)
        .expect_err("over-budget reads must fail");
    assert_eq!(error.to_string(), ARCHIVE_TOO_LARGE_MESSAGE);
    assert_eq!(out, b"hello");
}

#[test]
fn budget_reader_never_returns_more_than_the_remaining_budget() {
    use std::io::Read;

    let flag = std::sync::atomic::AtomicBool::new(false);
    let mut reader = budget_reader(b"hello world", 3, &flag);
    let mut buf = [0u8; 100];
    assert_eq!(reader.read(&mut buf).expect("capped read"), 3);
    assert_eq!(&buf[..3], &b"hello world"[..3]);
}

#[test]
fn budget_reader_checks_cancellation_on_every_read() {
    use std::io::Read;

    let flag = std::sync::atomic::AtomicBool::new(true);
    let mut reader = budget_reader(b"hello world", MAX_TAR_GZ_DECOMPRESSED_BYTES, &flag);
    let mut buf = [0u8; 4];
    let error = reader
        .read(&mut buf)
        .expect_err("cancelled reads must fail");
    assert_eq!(error.to_string(), "Preview cancelled");
}

#[test]
fn archive_wire_contract_round_trips_all_statuses() {
    use crate::services::ArchiveFileEntry;

    let entries = vec![ArchiveFileEntry {
        name: "folder/item.txt".to_owned(),
        directory: false,
        size: 8,
    }];
    for status in [
        ArchiveListingStatus::Open,
        ArchiveListingStatus::NeedsPassword,
        ArchiveListingStatus::WrongPassword,
        ArchiveListingStatus::Unsupported,
    ] {
        let listing = super::ArchiveListing {
            status,
            entries: entries.clone(),
        };
        let decoded =
            decode_archive_listing(&encode_archive_result(&Ok(listing))).expect("round trip");
        assert_eq!(decoded.status, status);
        assert_eq!(decoded.entries, entries);
    }
}

#[test]
fn archive_payload_validation_accepts_every_known_status() {
    use super::{WireStatus, archive_payload_valid, decode_archive_payload};

    for payload in [
        encode_archive_result(&Err(ARCHIVE_TOO_LARGE_MESSAGE.to_owned())),
        encode_archive_result(&Err(super::INVALID_ARCHIVE.to_owned())),
    ] {
        assert!(archive_payload_valid(&payload));
        let decoded = decode_archive_payload(&payload).expect("valid payload");
        assert!(decoded.entries.is_empty());
    }
    let too_large = decode_archive_payload(&encode_archive_result(&Err(
        ARCHIVE_TOO_LARGE_MESSAGE.to_owned()
    )))
    .expect("valid payload");
    assert_eq!(too_large.status, WireStatus::TooLarge);
    assert!(too_large.message.is_none());
    assert!(!archive_payload_valid(b"not json"));
    assert!(!archive_payload_valid(
        b"{\"status\":\"bogus\",\"entries\":[]}"
    ));
}

#[test]
fn archive_wire_contract_carries_limit_errors_without_entries() {
    match decode_archive_listing(&encode_archive_result(&Err(
        ARCHIVE_TOO_LARGE_MESSAGE.to_owned()
    ))) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("limit errors decode to errors"),
    }
}

#[test]
fn archive_wire_contract_rejects_malformed_payloads() {
    assert!(decode_archive_listing(b"not json").is_err());
    assert!(decode_archive_listing(b"{\"status\":\"open\"}").is_err());
    assert!(decode_archive_listing(b"{\"status\":\"nope\",\"entries\":[]}").is_err());
    assert!(
        decode_archive_listing(b"{\"status\":\"error\",\"entries\":[],\"message\":\"\"}").is_err()
    );
    assert!(
        decode_archive_listing(
            b"{\"status\":\"open\",\"entries\":[{\"name\":1,\"directory\":false,\"size\":1}]}"
        )
        .is_err()
    );
    assert!(
        decode_archive_listing(
            b"{\"status\":\"open\",\"entries\":[{\"name\":\"a\",\"directory\":false,\"size\":-1}]}"
        )
        .is_err()
    );
}

#[test]
fn archive_wire_contract_rejects_over_cap_payloads() {
    let entries = vec![
        serde_json::json!({"name": "f.txt", "directory": false, "size": 1 });
        MAX_ARCHIVE_ENTRIES + 1
    ];
    let payload = serde_json::json!({"status": "open", "entries": entries, "message": null})
        .to_string()
        .into_bytes();
    assert!(decode_archive_listing(&payload).is_err());
}

#[test]
fn archive_wire_contract_rejects_pathological_names_as_too_large() {
    let pathological = [
        "a/".repeat(10_000) + "file.txt",
        "ab/".repeat(5000) + "opaque.txt",
        "x".repeat(20_000) + "blob.txt",
    ];
    for name in pathological {
        let listing = super::ArchiveListing {
            status: ArchiveListingStatus::Open,
            entries: vec![crate::services::ArchiveFileEntry {
                name,
                directory: false,
                size: 1,
            }],
        };
        let payload = encode_archive_result(&Ok(listing));
        assert!(super::archive_payload_valid(&payload));
        match decode_archive_listing(&payload) {
            Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
            Ok(listing) => panic!("pathological name must not list: {:?}", listing.entries),
        }
    }
}

#[test]
fn archive_wire_contract_rejects_excessive_cumulative_names_as_too_large() {
    let prefix = "a/".repeat(400);
    let entries = (0..MAX_ARCHIVE_ENTRIES)
        .map(|index| crate::services::ArchiveFileEntry {
            name: format!("{prefix}f{index:05}.txt"),
            directory: false,
            size: 1,
        })
        .collect();
    let listing = super::ArchiveListing {
        status: ArchiveListingStatus::Open,
        entries,
    };
    let payload = encode_archive_result(&Ok(listing));
    assert!(super::archive_payload_valid(&payload));
    match decode_archive_listing(&payload) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(listing) => panic!(
            "over-budget names must not list: {} entries decoded",
            listing.entries.len()
        ),
    }
}

#[test]
fn tar_listing_rejects_pathological_names_as_too_large() {
    // Stay below the byte cap to exercise the independent segment cap.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("deep.tar");
    let long = "ab/".repeat(5000) + "file.txt";
    {
        let file = std::fs::File::create(&path).expect("create tar");
        let mut builder = tar::Builder::new(file);
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, &long, &b"hi"[..])
            .expect("append long name");
        builder.into_inner().expect("finish tar");
    }
    match list_archive_entries_direct(&path, ArchiveFormat::Tar, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("pathological tar name must not list"),
    }
}

#[test]
fn empty_zip_and_tar_archives_list_no_members() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = dir.path().join("empty.zip");
    write_zip_stored(&zip, &[]).expect("write zip");
    assert_eq!(list_entries(&zip, ArchiveFormat::Zip), Vec::new());
    let tar = dir.path().join("empty.tar");
    std::fs::write(&tar, b"").expect("write tar");
    assert_eq!(list_entries(&tar, ArchiveFormat::Tar), Vec::new());
}

#[test]
fn empty_sevenz_input_fails_without_a_listing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("empty.7z");
    std::fs::write(&path, b"").expect("write 7z");
    assert!(
        list_archive_entries_direct(&path, ArchiveFormat::SevenZ, None, &never_cancelled())
            .is_err(),
        "empty 7z input must fail, not list"
    );
}

#[test]
fn tar_gz_compressed_gate_allows_exactly_one_gib() {
    use super::MAX_TAR_GZ_COMPRESSED_BYTES;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("exact.tar.gz");
    std::fs::File::create(&path)
        .expect("create sparse input")
        .set_len(MAX_TAR_GZ_COMPRESSED_BYTES)
        .expect("size sparse input");
    match list_archive_entries_direct(&path, ArchiveFormat::TarGz, None, &never_cancelled()) {
        Err(message) => assert_ne!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("zeros are not a valid gzip stream"),
    }
}

#[test]
fn gnu_long_names_list_with_their_full_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("long.tar");
    let long = "d/".repeat(60) + "file.txt";
    {
        let file = std::fs::File::create(&path).expect("create tar");
        let mut builder = tar::Builder::new(file);
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, &long, &b"hi"[..])
            .expect("append long name");
        builder.into_inner().expect("finish tar");
    }
    assert_eq!(
        list_entries(&path, ArchiveFormat::Tar),
        vec![(long, false, 2)]
    );
}

#[test]
fn tar_gz_listing_rejects_decompressed_output_over_budget() {
    use super::MAX_TAR_GZ_DECOMPRESSED_BYTES;
    use std::io::Read as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bomb.tar.gz");
    {
        let file = std::fs::File::create(&path).expect("create tar.gz");
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(MAX_TAR_GZ_DECOMPRESSED_BYTES + 1);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        builder
            .append_data(
                &mut header,
                "big.bin",
                std::io::repeat(0).take(MAX_TAR_GZ_DECOMPRESSED_BYTES + 1),
            )
            .expect("append bomb member");
        builder
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip");
    }
    match list_archive_entries_direct(&path, ArchiveFormat::TarGz, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, ARCHIVE_TOO_LARGE_MESSAGE),
        Ok(_) => panic!("decompression bomb must not list"),
    }
}

#[test]
fn archive_wire_contract_preserves_generic_error_messages() {
    let message = super::INVALID_ARCHIVE.to_owned();
    match decode_archive_listing(&encode_archive_result(&Err(message.clone()))) {
        Err(decoded) => assert_eq!(decoded, message),
        Ok(_) => panic!("error payloads decode to errors"),
    }
}

#[test]
fn archive_wire_contract_ignores_unknown_fields() {
    let payload = serde_json::json!({
        "status": "open",
        "entries": [{"name": "a.txt", "directory": false, "size": 1, "future": true}],
        "message": null,
        "future": {"nested": [1, 2, 3]},
    })
    .to_string()
    .into_bytes();
    let listing = decode_archive_listing(&payload).expect("tolerant decode");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "a.txt");
}

#[test]
fn malformed_inputs_normalize_to_invalid_archive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tar = dir.path().join("junk.tar");
    std::fs::write(&tar, b"this is definitely not an archive file....").expect("write junk");
    match list_archive_entries_direct(&tar, ArchiveFormat::Tar, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("junk tar must fail"),
    }
    let gzip = dir.path().join("junk.tar.gz");
    std::fs::write(&gzip, b"this is definitely not an archive file....").expect("write junk");
    match list_archive_entries_direct(&gzip, ArchiveFormat::TarGz, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("junk tar.gz must fail"),
    }
    let sevenz = dir.path().join("junk.7z");
    std::fs::write(&sevenz, b"this is definitely not an archive file....").expect("write junk");
    match list_archive_entries_direct(&sevenz, ArchiveFormat::SevenZ, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("junk 7z must fail"),
    }
}

#[test]
fn truncated_archives_fail_without_listings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = dir.path().join("cut.zip");
    write_zip_stored(&zip, &[("item.txt", b"contents-data-here")]).expect("write zip");
    let bytes = std::fs::read(&zip).expect("read zip");
    std::fs::write(&zip, &bytes[..bytes.len() / 2]).expect("truncate zip");
    match list_archive_entries_direct(&zip, ArchiveFormat::Zip, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("truncated zip must fail"),
    }
    let sevenz = dir.path().join("cut.7z");
    write_7z(&sevenz, "item.txt", b"contents-data-here").expect("write 7z");
    let bytes = std::fs::read(&sevenz).expect("read 7z");
    std::fs::write(&sevenz, &bytes[..bytes.len() / 2]).expect("truncate 7z");
    match list_archive_entries_direct(&sevenz, ArchiveFormat::SevenZ, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("truncated 7z must fail"),
    }
    let gzip = dir.path().join("cut.tar.gz");
    write_tar(&gzip, "item.txt", b"contents-data-here", true).expect("write tar.gz");
    let bytes = std::fs::read(&gzip).expect("read tar.gz");
    std::fs::write(&gzip, &bytes[..bytes.len() / 2]).expect("truncate tar.gz");
    match list_archive_entries_direct(&gzip, ArchiveFormat::TarGz, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("truncated tar.gz must fail"),
    }
}

#[test]
fn invalid_tar_inside_valid_gzip_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("badtar.tar.gz");
    {
        use std::io::Write;
        let file = std::fs::File::create(&path).expect("create tar.gz");
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder
            .write_all(b"definitely not a tar stream")
            .expect("write payload");
        encoder.finish().expect("finish gzip");
    }
    match list_archive_entries_direct(&path, ArchiveFormat::TarGz, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::INVALID_ARCHIVE),
        Ok(_) => panic!("invalid tar inside valid gzip must fail"),
    }
}

#[test]
#[ignore = "manual benchmark; run explicitly, not in CI"]
fn manual_benchmark_flat_zip_20k_pipeline() {
    use std::time::Instant;

    let dir = tempfile::tempdir().expect("tempdir");
    let names: Vec<String> = (0..20_000)
        .map(|index| format!("file-{index:05}.txt"))
        .collect();
    let entries: Vec<(&str, &[u8])> = names
        .iter()
        .map(|name| (name.as_str(), b"x".as_slice()))
        .collect();
    let path = dir.path().join("flat-20000.zip");
    let started = Instant::now();
    write_zip_stored(&path, &entries).expect("write fixture");
    eprintln!(
        "benchmark: fixture (20k flat zip) took {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    let listing = list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled())
        .expect("listing should succeed");
    assert_eq!(listing.entries.len(), 20_000);
    eprintln!("benchmark: list took {:?}", started.elapsed());

    let started = Instant::now();
    let bytes = encode_archive_result(&Ok(listing));
    eprintln!(
        "benchmark: encode took {:?} ({} bytes)",
        started.elapsed(),
        bytes.len()
    );

    let started = Instant::now();
    let decoded = decode_archive_listing(&bytes).expect("decode should succeed");
    eprintln!("benchmark: decode took {:?}", started.elapsed());

    let started = Instant::now();
    let tree = crate::services::archive_preview_tree(decoded.entries);
    assert_eq!(tree.file_count, 20_000);
    eprintln!("benchmark: tree took {:?}", started.elapsed());
}

#[test]
fn zip_error_sorts_unsupported_methods_from_corruption() {
    assert_eq!(
        super::zip_error(zip::result::ZipError::UnsupportedArchive("nope")),
        super::ARCHIVE_UNSUPPORTED_MESSAGE
    );
    assert_eq!(
        super::zip_error(zip::result::ZipError::CompressionMethodNotSupported(100)),
        super::ARCHIVE_UNSUPPORTED_MESSAGE
    );
    for error in [
        zip::result::ZipError::InvalidArchive("nope".into()),
        zip::result::ZipError::FileNotFound,
        zip::result::ZipError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "truncated",
        )),
    ] {
        assert_eq!(
            super::zip_error(error),
            super::INVALID_ARCHIVE,
            "non-feature ZIP failures are state 5"
        );
    }
}

#[test]
fn unsupported_zip_extra_reports_unsupported_format() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("aes.zip");
    {
        let file = std::fs::File::create(&path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "secret.txt",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored)
                    .with_aes_encryption(zip::AesMode::Aes256, "pw"),
            )
            .expect("start entry");
        writer.write_all(b"topsecret").expect("write entry");
        writer.finish().expect("finish zip");
    }
    let mut bytes = std::fs::read(&path).expect("read zip");
    let mut patched = 0;
    let mut index = 0;
    while index + 4 <= bytes.len() {
        // WinZip AES extra-field ID; its length must be 7.
        if bytes[index] == 0x01 && bytes[index + 1] == 0x99 {
            bytes[index + 2..index + 4].copy_from_slice(&5u16.to_le_bytes());
            patched += 1;
        }
        index += 1;
    }
    assert!(patched >= 2, "expected local and central AES extra fields");
    std::fs::write(&path, &bytes).expect("patch extra");
    match list_archive_entries_direct(&path, ArchiveFormat::Zip, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::ARCHIVE_UNSUPPORTED_MESSAGE),
        Ok(_) => panic!("unparsable-extra zip must fail"),
    }
}

#[test]
fn sevenz_unsupported_error_reports_unsupported_format() {
    assert_eq!(
        super::sevenz_list_error(sevenz_rust2::Error::Unsupported("nope".into())),
        super::ARCHIVE_UNSUPPORTED_MESSAGE
    );
    for error in [
        sevenz_rust2::Error::BadSignature([0; 6]),
        sevenz_rust2::Error::ChecksumVerificationFailed,
        sevenz_rust2::Error::ExternalUnsupported,
        sevenz_rust2::Error::UnsupportedCompressionMethod("nope".to_owned()),
        sevenz_rust2::Error::Other("Cannot handle next_header_size 60".into()),
    ] {
        assert_eq!(
            super::sevenz_list_error(error),
            super::INVALID_ARCHIVE,
            "7z failures other than Error::Unsupported are state 5"
        );
    }
}

#[test]
fn rar_archives_report_unsupported_format() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.rar");
    std::fs::write(&path, b"rarrr").expect("write junk");
    match list_archive_entries_direct(&path, ArchiveFormat::Rar, None, &never_cancelled()) {
        Err(message) => assert_eq!(message, super::ARCHIVE_UNSUPPORTED_MESSAGE),
        Ok(_) => panic!("rar must fail"),
    }
}
