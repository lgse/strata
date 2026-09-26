// SPDX-License-Identifier: MIT

use super::*;

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
