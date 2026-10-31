// SPDX-License-Identifier: MIT

use super::*;
use gtk::glib;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn stream_error_reports_cancelled_when_the_flag_is_set_regardless_of_message() {
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        stream_error(Failure::from("some unrelated failure"), None, &cancelled),
        ArchiveError::Cancelled
    );
    assert_eq!(
        stream_error(
            Failure::from("Operation cancelled"),
            Some(ArchiveError::PasswordRequired("member".to_owned())),
            &cancelled
        ),
        ArchiveError::Cancelled
    );
}

#[test]
fn stream_error_keeps_the_failure_kind_when_not_cancelled() {
    let cancelled = AtomicBool::new(false);
    let required = "A password is required to extract this archive.";
    let incorrect = "The password may be incorrect.";
    let invalid = "This file is not a valid archive or is damaged.";
    for (failure, member_error, expected) in [
        (
            Failure::from(invalid),
            None,
            ArchiveError::Failed(invalid.to_owned()),
        ),
        (
            Failure::new(FailureKind::PasswordRequired, required),
            None,
            ArchiveError::PasswordRequired(required.to_owned()),
        ),
        (
            Failure::new(FailureKind::IncorrectPassword, incorrect),
            None,
            ArchiveError::IncorrectPassword(incorrect.to_owned()),
        ),
        // A member error reaches the stream only as text; the session's original wins.
        (
            Failure::from(incorrect),
            Some(ArchiveError::IncorrectPassword(incorrect.to_owned())),
            ArchiveError::IncorrectPassword(incorrect.to_owned()),
        ),
    ] {
        assert_eq!(
            stream_error(failure.clone(), member_error, &cancelled),
            expected,
            "{failure:?}"
        );
    }
}

#[test]
fn a_failed_member_trailer_keeps_its_kind_through_the_member_body() {
    let required = Failure::new(
        FailureKind::PasswordRequired,
        "A password is required to extract this archive.",
    );
    let mut stream = Vec::new();
    crate::rar_extraction::write_file_failed(&mut stream, &required).expect("fixture stream");
    let mut reader = std::io::Cursor::new(stream);
    let mut body = crate::rar_extraction::FileBody::new(&mut reader, 4);
    let error = MemberBody(&mut body)
        .read(&mut [0; 4])
        .expect_err("a failed trailer must fail the read");
    assert_eq!(
        crate::adapters::local_operations::archive::archive_read_failed(error),
        ArchiveError::PasswordRequired(required.message)
    );
}

#[test]
fn wire_metadata_maps_to_member_metadata() {
    let at = |seconds| Some(UNIX_EPOCH + Duration::from_secs(seconds));
    let dos_local = glib::DateTime::from_local(2015, 8, 7, 17, 21, 8.0)
        .expect("valid local time")
        .to_unix();
    for (mode, modified, expected) in [
        (None, None, None),
        // 2015-08-07 17:21:08 UTC as a Windows FILETIME.
        (
            Some(0o100_644),
            Some(WireTime::FileTime(130_834_416_680_000_000)),
            at(1_438_968_068),
        ),
        // The same wall-clock time as a DOS date and time, read as local time.
        (
            None,
            Some(WireTime::DosLocal(0x4707_8AA4)),
            at(dos_local.try_into().expect("after 1970")),
        ),
        (None, Some(WireTime::FileTime(1)), None),
    ] {
        assert_eq!(
            member_metadata(WireMetadata { mode, modified }, false),
            MemberMetadata {
                mode,
                modified: expected,
            }
        );
    }
    for (mode, directory, applied) in [
        (0o040_755, true, true),
        (0o040_755, false, false),
        (0o100_644, true, false),
        (0o120_777, false, false),
    ] {
        let wire = WireMetadata {
            mode: Some(mode),
            modified: None,
        };
        assert_eq!(
            member_metadata(wire, directory).mode,
            applied.then_some(mode),
            "{mode:o}"
        );
    }
}
