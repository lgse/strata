// SPDX-License-Identifier: MIT

//! The preview helper's `archive-list` operation, run in process over the real archive reader.

use strata_preview::sandbox_helper;

fn archive_list_output(
    directory: &tempfile::TempDir,
    input: &std::path::Path,
    format: &str,
    password: Option<&[u8]>,
) -> Vec<u8> {
    let output = directory.path().join("result.archive.json");
    let secret =
        password.map(|password| crate::sandbox::stage_secret_anon(password).expect("stage secret"));
    let mut arguments = vec![
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        format.to_owned(),
        "software".to_owned(),
    ];
    if let Some(secret) = &secret {
        use std::os::fd::AsRawFd;
        arguments.push(secret.as_raw_fd().to_string());
    }
    sandbox_helper::install_archive_lister(crate::adapters::list_archive_entries_direct);
    sandbox_helper::run(&arguments).expect("helper runs");
    std::fs::read(&output).expect("read listing output")
}

fn decoded_archive_list(output: &[u8]) -> Result<crate::adapters::ArchiveListing, String> {
    crate::adapters::decode_archive_listing(output)
}

fn write_tar_fixture(path: &std::path::Path, members: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("create tar");
    let mut builder = tar::Builder::new(file);
    for (name, contents) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, name, *contents)
            .expect("append member");
    }
    builder.into_inner().expect("finish tar");
}

fn write_encrypted_zip_fixture(
    directory: &tempfile::TempDir,
    name: &str,
    password: &str,
) -> std::path::PathBuf {
    use crate::{adapters::write_compression_fixture, services::ArchiveFormat};

    let source = directory.path().join("folder");
    std::fs::create_dir_all(&source).expect("create source");
    std::fs::write(source.join("item.txt"), b"contents").expect("write source");
    let path = directory.path().join(name);
    write_compression_fixture(&path, &[source], ArchiveFormat::Zip, Some(password))
        .expect("write encrypted zip");
    path
}

#[test]
fn archive_list_lists_plain_tar_members() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(
        &input,
        &[("readme.txt", b"hi"), ("src/main.rs", b"fn main(){}")],
    );
    let output = archive_list_output(&directory, &input, "tar", None);
    let listing = decoded_archive_list(&output).expect("decode listing");
    assert_eq!(listing.status, ArchiveListingStatus::Open);
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>(),
        vec!["readme.txt".to_owned(), "src/main.rs".to_owned()]
    );
}

#[test]
fn archive_list_needs_password_without_one() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let input = write_encrypted_zip_fixture(&directory, "secret.zip", "s3cret");
    let output = archive_list_output(&directory, &input, "zip", None);
    let listing = decoded_archive_list(&output).expect("decode listing");
    assert_eq!(listing.status, ArchiveListingStatus::NeedsPassword);
    assert!(
        !listing.entries.is_empty(),
        "encrypted zip names stay listable without a password"
    );
}

#[test]
fn archive_list_unlocks_with_staged_password() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let password = "päss wörd $HOME `id`!";
    let input = write_encrypted_zip_fixture(&directory, "secret.zip", password);
    let output = archive_list_output(&directory, &input, "zip", Some(password.as_bytes()));
    let listing = decoded_archive_list(&output).expect("decode listing");
    assert_eq!(listing.status, ArchiveListingStatus::Open);
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.name == "folder/item.txt"),
        "unicode/spaces/metacharacter password round-trips byte-exactly"
    );
}

#[test]
fn archive_list_rejects_wrong_and_empty_passwords() {
    use crate::adapters::ArchiveListingStatus;

    let directory = tempfile::tempdir().expect("tempdir");
    let input = write_encrypted_zip_fixture(&directory, "secret.zip", "s3cret");
    for password in ["wrong", ""] {
        let output = archive_list_output(&directory, &input, "zip", Some(password.as_bytes()));
        let listing = decoded_archive_list(&output).expect("decode listing");
        assert_eq!(
            listing.status,
            ArchiveListingStatus::WrongPassword,
            "password {password:?} must be rejected"
        );
    }
}

#[test]
fn archive_list_rejects_unknown_format() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let error = sandbox_helper::run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "rar".to_owned(),
        "software".to_owned(),
    ])
    .expect_err("unknown format must fail");
    assert_eq!(error, "Unknown archive format for preview.");
    assert!(
        !output.exists(),
        "rejected formats must not produce an output file"
    );
}

#[test]
fn archive_list_rejects_overlong_password() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let secret = crate::sandbox::stage_secret_anon(&vec![
        b'x';
        crate::sandbox::archive_listing::MAX_ARCHIVE_PASSWORD_BYTES
            + 1
    ])
    .expect("stage password");
    use std::os::fd::AsRawFd;
    let error = sandbox_helper::run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "tar".to_owned(),
        "software".to_owned(),
        secret.as_raw_fd().to_string(),
    ])
    .expect_err("overlong password must fail");
    assert_eq!(error, "Archive password is too long.");
}

#[test]
fn archive_list_rejects_non_utf8_password() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let secret = crate::sandbox::stage_secret_anon(&[0xFF, 0xFE]).expect("stage password");
    use std::os::fd::AsRawFd;
    let error = sandbox_helper::run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "tar".to_owned(),
        "software".to_owned(),
        secret.as_raw_fd().to_string(),
    ])
    .expect_err("non-UTF8 password must fail");
    assert_eq!(error, "Archive password is not valid text.");
}

#[test]
fn archive_list_rejects_an_unreadable_secret_descriptor() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    let number = i32::MAX;
    let error = sandbox_helper::run(&[
        "archive-list".to_owned(),
        input.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "tar".to_owned(),
        "software".to_owned(),
        number.to_string(),
    ])
    .expect_err("unreadable secret must fail");
    assert!(
        error.starts_with("Unable to read the preview secret: "),
        "unexpected error: {error:?}"
    );
    assert!(
        !output.exists(),
        "failed listings must not produce an output file, got {error:?}"
    );
}

#[test]
fn archive_list_rejects_a_malformed_secret_descriptor() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("docs.tar");
    write_tar_fixture(&input, &[("readme.txt", b"hi")]);
    let output = directory.path().join("result.archive.json");
    for descriptor in ["not-a-number", "-1"] {
        let error = sandbox_helper::run(&[
            "archive-list".to_owned(),
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            "tar".to_owned(),
            "software".to_owned(),
            descriptor.to_owned(),
        ])
        .expect_err("malformed descriptor must fail");
        assert_eq!(error, "Invalid preview helper secret descriptor");
    }
}

#[test]
fn archive_list_rejects_a_trailing_argument_on_other_operations() {
    let error = sandbox_helper::run(&[
        "preview-image".to_owned(),
        "/tmp/input".to_owned(),
        "/tmp/output.png".to_owned(),
        "800".to_owned(),
        "software".to_owned(),
        "3".to_owned(),
    ])
    .expect_err("extra argument must fail");
    assert_eq!(error, "Invalid preview helper arguments");
}

#[test]
fn archive_list_reports_corrupt_input_without_crashing() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("junk.zip");
    std::fs::write(&input, b"not an archive at all").expect("write junk");
    let output = archive_list_output(&directory, &input, "zip", None);
    match decoded_archive_list(&output) {
        Err(message) => assert_eq!(message, crate::sandbox::archive_listing::INVALID_ARCHIVE),
        Ok(_) => panic!("corrupt input must not list"),
    }
}
