// SPDX-License-Identifier: MIT

use super::*;
use crate::rar_extraction::{self as wire, Record};
use std::io::Cursor;

// local_operations::archive's own fixtures.rs cannot be reached from here
// (both `local_operations` and `archive` are private modules), so these are
// their own copies of the same physical files.
const RAR_VERSION_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/rar/version.rar");
const RAR_ENCRYPTED_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/rar/encrypted.rar");

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
    let mut body = [0u8; 11];
    std::io::Read::read_exact(&mut reader, &mut body).expect("real RAR fixture");
    assert_eq!(&body, b"unrar-0.4.0");
    assert_eq!(
        wire::read_file_trailer(&mut reader).expect("real RAR fixture"),
        Ok(())
    );
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::End
    );
}

// RAR_ENCRYPTED_FIXTURE has plain (unencrypted) headers, only its content is
// encrypted — matching real archives, UnRAR asks for the password via the
// UCM_NEEDPASSWORD callback before it ever emits a UCM_PROCESSDATA byte for
// that member, so a missing/wrong password never reaches the sink at all.
// `run()` itself still returns `Ok(())`: one member failing and reporting
// that through its own trailer is, from the stream's own perspective, a
// cleanly finished session — see `extract()`'s comment at its file-failure
// branch. The archive-open-time error path (failing before any member
// header at all) is covered separately by `run_reports_a_corrupt_archive`.
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
    assert_eq!(
        wire::read_file_trailer(&mut reader).expect("real RAR fixture"),
        Err("A password is required to extract this archive.".to_owned())
    );
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
    assert_eq!(
        wire::read_file_trailer(&mut reader).expect("real RAR fixture"),
        Err(MAYBE_BAD_PASSWORD.to_owned())
    );
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
    let mut body = [0u8; 18];
    std::io::Read::read_exact(&mut reader, &mut body).expect("real RAR fixture");
    assert_eq!(&body, b"target\nCargo.lock\n");
    assert_eq!(
        wire::read_file_trailer(&mut reader).expect("real RAR fixture"),
        Ok(())
    );
    assert_eq!(
        wire::read_record(&mut reader).expect("real RAR fixture"),
        Record::End
    );
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
