// SPDX-License-Identifier: MIT

use super::*;
use crate::rar_extraction::{self as wire, Record};
use std::io::Cursor;

const RAR_VERSION_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/rar/version.rar");
const RAR_ENCRYPTED_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/rar/encrypted.rar");
const RAR_ENCRYPTED_HEADERS_FIXTURE: &[u8] =
    include_bytes!("../../../tests/fixtures/rar/comment-hpw-password.rar");
const RAR_UNICODE_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/rar/unicode.rar");

#[test]
fn excessive_native_dictionary_is_rejected() {
    let result = call(None, None, |state| {
        assert_eq!(
            callback(UCM_LARGEDICT, state, 2 * 1024 * 1024, 1024 * 1024),
            -1
        );
        ERAR_LARGE_DICT
    });
    assert_eq!(
        result.expect_err("large dictionary callback must fail"),
        LARGE_DICTIONARY
    );
    assert_eq!(
        decode_result(ERAR_LARGE_DICT, None).expect_err("large dictionary code must fail"),
        LARGE_DICTIONARY
    );
}

#[test]
fn callback_streams_bounded_chunks_to_a_sink() {
    let chunk = [42u8; 65536];
    let mut received = Vec::new();
    let mut sink = |bytes: &[u8]| {
        received.extend_from_slice(bytes);
        Ok(())
    };
    call(None, Some(&mut sink), |state| {
        for _ in 0..1024 {
            assert_eq!(
                callback(
                    native::UCM_PROCESSDATA,
                    state,
                    chunk.as_ptr() as native::LPARAM,
                    chunk.len() as native::LPARAM
                ),
                1
            );
        }
        0
    })
    .expect("real RAR fixture");
    assert_eq!(received.len(), 1024 * 65536);
}

fn write_fixture(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("fixture directory");
    let path = dir.path().join("archive.rar");
    std::fs::write(&path, bytes).expect("fixture archive");
    (dir, path)
}

#[test]
fn run_streams_the_version_fixture_in_wire_format() {
    let (_dir, archive) = write_fixture(RAR_VERSION_FIXTURE);
    let mut buffer = Vec::new();
    run(&archive, None, &mut buffer).expect("streaming a valid archive must succeed");

    let mut reader = Cursor::new(buffer);
    wire::read_magic(&mut reader).expect("stream must start with the wire magic");
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::File("VERSION".to_owned(), 11)
    );
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut wire::FileBody::new(&mut reader, 11), &mut body)
        .expect("real RAR fixture");
    assert_eq!(&body, b"unrar-0.4.0");
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::End
    );
}

#[test]
fn cover_image_fails_for_a_rar_without_images() {
    let (_dir, archive) = write_fixture(RAR_VERSION_FIXTURE);
    assert_eq!(
        cover_image(&archive).expect_err("archive has no cover"),
        "Comic archive has no bounded image"
    );
}

#[test]
fn run_reports_a_missing_password_as_a_file_trailer_failure() {
    let (_dir, archive) = write_fixture(RAR_ENCRYPTED_FIXTURE);
    let mut buffer = Vec::new();
    run(&archive, None, &mut buffer)
        .expect("a per-member trailer failure is not a top-level stream error");

    let mut reader = Cursor::new(buffer);
    wire::read_magic(&mut reader).expect("real RAR fixture");
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::File(".gitignore".to_owned(), 18)
    );
    let mut body = Vec::new();
    let error = std::io::Read::read_to_end(&mut wire::FileBody::new(&mut reader, 18), &mut body)
        .expect_err("missing password must fail before yielding contents");
    assert_eq!(
        error.to_string(),
        "A password is required to extract this archive."
    );
    assert!(body.is_empty());
}

#[test]
fn run_reports_a_wrong_password_as_a_file_trailer_failure() {
    let (_dir, archive) = write_fixture(RAR_ENCRYPTED_FIXTURE);
    let mut buffer = Vec::new();
    run(&archive, Some("wrong-password"), &mut buffer)
        .expect("a per-member trailer failure is not a top-level stream error");

    let mut reader = Cursor::new(buffer);
    wire::read_magic(&mut reader).expect("real RAR fixture");
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::File(".gitignore".to_owned(), 18)
    );
    let mut body = Vec::new();
    let error = std::io::Read::read_to_end(&mut wire::FileBody::new(&mut reader, 18), &mut body)
        .expect_err("wrong password must fail before yielding contents");
    assert_eq!(error.to_string(), MAYBE_BAD_PASSWORD);
    assert!(body.is_empty());
}

#[test]
fn run_streams_an_encrypted_archive_with_the_correct_password() {
    let (_dir, archive) = write_fixture(RAR_ENCRYPTED_FIXTURE);
    let mut buffer = Vec::new();
    run(&archive, Some("unrar"), &mut buffer).expect("the correct password must succeed");

    let mut reader = Cursor::new(buffer);
    wire::read_magic(&mut reader).expect("real RAR fixture");
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::File(".gitignore".to_owned(), 18)
    );
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut wire::FileBody::new(&mut reader, 18), &mut body)
        .expect("real RAR fixture");
    assert_eq!(&body, b"target\nCargo.lock\n");
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::End
    );
}

#[test]
fn run_handles_encrypted_headers() {
    let (_dir, archive) = write_fixture(RAR_ENCRYPTED_HEADERS_FIXTURE);
    for (password, expected) in [
        (None, "A password is required to extract this archive."),
        (Some("wrong-password"), MAYBE_BAD_PASSWORD),
    ] {
        let mut buffer = Vec::new();
        run(&archive, password, &mut buffer).expect_err("encrypted headers require a password");
        let mut reader = Cursor::new(buffer);
        wire::read_magic(&mut reader).expect("stream magic");
        assert_eq!(
            wire::read_record(&mut reader).expect("error record"),
            Record::Error(expected.to_owned())
        );
    }
    let mut buffer = Vec::new();
    run(&archive, Some("password"), &mut buffer).expect("correct password");
    let mut reader = Cursor::new(buffer);
    wire::read_magic(&mut reader).expect("stream magic");
    assert_eq!(
        wire::read_record(&mut reader).expect("member"),
        Record::File(".gitignore".to_owned(), 18)
    );
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut wire::FileBody::new(&mut reader, 18), &mut body)
        .expect("member contents");
    assert_eq!(body, b"target\nCargo.lock\n");
}

#[test]
fn run_handles_unicode_member_names() {
    let (_dir, archive) = write_fixture(RAR_UNICODE_FIXTURE);
    let mut buffer = Vec::new();
    run(&archive, None, &mut buffer).expect("unicode archive");
    let mut reader = Cursor::new(buffer);
    wire::read_magic(&mut reader).expect("stream magic");
    let mut names = Vec::new();
    loop {
        match wire::read_record(&mut reader).expect("member record") {
            Record::File(name, size) => {
                names.push(name);
                std::io::copy(
                    &mut wire::FileBody::new(&mut reader, size),
                    &mut std::io::sink(),
                )
                .expect("member contents");
            }
            Record::Directory(name) => names.push(name),
            Record::End => break,
            Record::Error(error) => panic!("unexpected error: {error}"),
        }
    }
    assert!(names.iter().any(|name| !name.is_ascii()));
}

#[test]
fn run_reports_a_corrupt_archive() {
    let (_dir, archive) = write_fixture(b"Rar!\x1a\x07\x00corrupt garbage data");
    let mut buffer = Vec::new();
    let error = run(&archive, None, &mut buffer).expect_err("a corrupt archive must fail");
    assert_eq!(error, INVALID_ARCHIVE);
}

#[test]
fn unrar_decode_error_mapping() {
    use unrar::error::{Code, UnrarError, When};

    let make_err = |code| UnrarError {
        code,
        when: When::Process,
    };

    assert_eq!(
        unrar_decode_error(make_err(Code::MissingPassword), false),
        "A password is required to extract this archive."
    );
    assert_eq!(
        unrar_decode_error(make_err(Code::BadPassword), true),
        MAYBE_BAD_PASSWORD
    );
    assert_eq!(
        unrar_decode_error(make_err(Code::BadData), true),
        MAYBE_BAD_PASSWORD
    );
    assert_eq!(
        unrar_decode_error(make_err(Code::BadData), false),
        INVALID_ARCHIVE
    );
    assert_eq!(
        unrar_decode_error(make_err(Code::BadArchive), false),
        INVALID_ARCHIVE
    );
    assert_eq!(
        unrar_decode_error(make_err(Code::UnknownFormat), false),
        INVALID_ARCHIVE
    );
}
