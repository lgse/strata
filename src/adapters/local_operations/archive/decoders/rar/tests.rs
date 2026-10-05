// SPDX-License-Identifier: MIT

use super::*;
use gtk::glib;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn stream_error_reports_cancelled_when_the_flag_is_set_regardless_of_message() {
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        stream_error("some unrelated failure".to_owned(), &cancelled),
        ArchiveError::Cancelled
    );
    assert_eq!(
        stream_error("Operation cancelled".to_owned(), &cancelled),
        ArchiveError::Cancelled
    );
}

#[test]
fn stream_error_reports_failed_with_the_message_when_not_cancelled() {
    let cancelled = AtomicBool::new(false);
    assert_eq!(
        stream_error(
            "This file is not a valid archive or is damaged.".to_owned(),
            &cancelled
        )
        .to_string(),
        "This file is not a valid archive or is damaged."
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
        // A FILETIME before 1970 is not applied.
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
    // Only a mode whose type matches the member applies; a RAR symlink
    // arrives as a file and keeps the default permissions.
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
