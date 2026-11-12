// SPDX-License-Identifier: MIT

//! Translation of decoder library errors into archive errors.

use super::super::{
    MAYBE_BAD_PASSWORD, PASSWORD_REQUIRED, UNSUPPORTED_ARCHIVE, archive_read_error,
    sevenz_decode_error, zip_error,
};
use super::*;
use crate::services::INCORRECT_ARCHIVE_PASSWORD;
use sevenz_rust2::Error as SevenZError;
use zip::result::ZipError;

#[test]
fn zip_errors_translate_to_unsupported_invalid_and_password_failures() {
    for (error, expected) in [
        (
            ZipError::UnsupportedArchive("unsupported encryption"),
            UNSUPPORTED_ARCHIVE,
        ),
        (
            ZipError::CompressionMethodNotSupported(99),
            UNSUPPORTED_ARCHIVE,
        ),
        (ZipError::FileNotFound, INVALID_ARCHIVE),
    ] {
        assert_eq!(zip_error(error).to_string(), expected);
    }
    assert_eq!(
        zip_error(ZipError::InvalidPassword),
        ArchiveError::IncorrectPassword(INCORRECT_ARCHIVE_PASSWORD.to_owned())
    );
    assert_eq!(
        zip_error(ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED)),
        ArchiveError::PasswordRequired(PASSWORD_REQUIRED.to_owned())
    );
    assert!(matches!(
        zip_error(ZipError::UnsupportedArchive("unsupported encryption")),
        ArchiveError::Failed(_)
    ));
}

#[test]
fn sevenz_errors_translate_to_password_unsupported_and_invalid_failures() {
    assert_eq!(
        sevenz_decode_error(SevenZError::PasswordRequired),
        ArchiveError::PasswordRequired(
            "A password is required to extract this archive.".to_owned()
        )
    );
    assert_eq!(
        sevenz_decode_error(SevenZError::MaybeBadPassword(
            io::ErrorKind::InvalidData.into()
        )),
        ArchiveError::IncorrectPassword("The password may be incorrect.".to_owned())
    );
    for error in [
        SevenZError::UnsupportedVersion { major: 9, minor: 0 },
        SevenZError::UnsupportedCompressionMethod("unknown".into()),
        SevenZError::Unsupported("unsupported encryption".into()),
    ] {
        assert_eq!(sevenz_decode_error(error).to_string(), UNSUPPORTED_ARCHIVE);
    }
    for error in [
        SevenZError::FileNotFound,
        SevenZError::Other("Broken or unsupported archive: no Header".into()),
        SevenZError::Other("Cannot handle next_header_size 576".into()),
    ] {
        assert_eq!(sevenz_decode_error(error).to_string(), INVALID_ARCHIVE);
    }
}

#[test]
fn read_errors_translate_by_whether_a_password_was_supplied() {
    for supplied in [false, true] {
        let translated = archive_read_error(io::ErrorKind::InvalidData.into(), supplied);
        assert_eq!(
            super::super::super::archive_read_failed(translated),
            if supplied {
                ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_owned())
            } else {
                ArchiveError::Failed(INVALID_ARCHIVE.to_owned())
            }
        );
    }
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
        io::ErrorKind::StorageFull,
        io::ErrorKind::Unsupported,
        io::ErrorKind::Interrupted,
        io::ErrorKind::InvalidInput,
        io::ErrorKind::Other,
    ] {
        let error = io::Error::new(kind, "injected I/O failure");
        let translated = archive_read_error(error, false);
        assert_eq!(translated.kind(), kind);
        assert_eq!(translated.to_string(), "injected I/O failure");
    }
}

#[test]
fn checksum_failure_without_a_password_remains_damaged() {
    let error = io::Error::other(sevenz_rust2::Error::ChecksumVerificationFailed);
    assert_eq!(
        archive_read_error(error, false).to_string(),
        INVALID_ARCHIVE
    );
}

#[test]
fn corrupt_deflate_stream_without_a_password_stays_damaged() {
    let error = io::Error::new(io::ErrorKind::InvalidInput, "corrupt deflate stream");
    assert_eq!(
        archive_read_error(error, false).to_string(),
        INVALID_ARCHIVE
    );
}

#[test]
fn corrupt_deflate_stream_after_a_password_is_retryable() {
    let error = io::Error::new(io::ErrorKind::InvalidInput, "corrupt deflate stream");
    assert_eq!(
        archive_read_error(error, true).to_string(),
        MAYBE_BAD_PASSWORD
    );
}
