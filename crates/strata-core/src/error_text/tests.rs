// SPDX-License-Identifier: MIT

use super::{error_detail_in, io_error_message};

#[test]
fn error_details_continue_in_lower_case_only_where_the_language_does() {
    for (locale, reason, expected) in [
        ("fr", "Délai dépassé", "délai dépassé"),
        ("ru", "Доступ запрещён", "доступ запрещён"),
        ("hu", "Hozzáférés megtagadva", "hozzáférés megtagadva"),
        (
            "vi",
            "Quyền truy cập bị từ chối",
            "quyền truy cập bị từ chối",
        ),
        ("pt-BR", "HTTP 404", "HTTP 404"),
        ("de", "Zugriff verweigert", "Zugriff verweigert"),
        ("en", "Permission denied", "Permission denied"),
    ] {
        assert_eq!(error_detail_in(locale, reason.to_owned()), expected);
    }
}

#[test]
fn io_errors_without_a_stable_kind_or_known_reason_get_catalog_text() {
    for (errno, expected) in [
        (libc::ELOOP, "There are too many levels of symbolic links"),
        (libc::ENXIO, "The device is not available"),
        (libc::ENOTCONN, "The location is no longer connected"),
        (libc::ETXTBSY, "The file is running and cannot be changed"),
        (libc::EXDEV, "The destination is on a different file system"),
        (libc::EPROTO, "An unexpected system error occurred"),
    ] {
        let error = std::io::Error::from_raw_os_error(errno);
        assert_eq!(io_error_message(&error), expected);
    }
    let custom = std::io::Error::new(std::io::ErrorKind::InvalidData, "broken stream");
    assert_eq!(io_error_message(&custom), "broken stream");
}
