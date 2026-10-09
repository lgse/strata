// SPDX-License-Identifier: MIT

use super::{
    LocationValidationError, UriCredentials, backend_unavailable_message, error_detail_in,
    io_error_message, sanitize_uri_credentials, validate_uri_credentials,
};

#[test]
fn error_details_continue_in_lower_case_only_where_the_language_does() {
    for (locale, reason, expected) in [
        ("fr", "Délai dépassé", "délai dépassé"),
        ("ru", "Доступ запрещён", "доступ запрещён"),
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
        (libc::EPROTO, "An unexpected system error occurred"),
    ] {
        let error = std::io::Error::from_raw_os_error(errno);
        assert_eq!(io_error_message(&error), expected);
    }
    let custom = std::io::Error::new(std::io::ErrorKind::InvalidData, "broken stream");
    assert_eq!(io_error_message(&custom), "broken stream");
}

#[test]
fn embedded_uri_credentials_are_rejected() {
    for uri in [
        "smb://user:secret@host/share",
        "smb://user%3Asecret@host/share",
        "smb://user:sec%72et@host/share",
        "smb://user:@host/share",
        "smb://user;password=secret@host/share",
        "smb://user%3Bpassword=secret@host/share",
        "smb://user%3Bpassword%3Dsecret@host/share",
        "smb://user;password=sec%72et@host/share",
        "smb://user;@host/share",
        "sftp://user:secret@host:2222/path",
        "ftp://user:secret@host/public",
    ] {
        assert_eq!(
            validate_uri_credentials(uri),
            Err(LocationValidationError::EmbeddedCredential),
            "{uri:?} should be rejected"
        );
    }
}

#[test]
fn embedded_uri_credentials_are_separated_from_the_sanitized_uri() {
    for (uri, sanitized) in [
        ("smb://user:secret@host/share", "smb://user@host/share"),
        ("smb://user%3Asecret@host/share", "smb://user@host/share"),
        (
            "smb://user;password=secret@host/share",
            "smb://user@host/share",
        ),
        (
            "smb://user%3Bpassword=secret@host/share",
            "smb://user@host/share",
        ),
        (
            "sftp://user%3Asecret@host/caf%E9%20x.txt",
            "sftp://user@host/caf%E9%20x.txt",
        ),
        (
            "smb://user:secret@host/a%2Fb?x=%FF#f",
            "smb://user@host/a%2Fb?x=%FF#f",
        ),
    ] {
        assert_eq!(
            sanitize_uri_credentials(uri),
            Ok((
                sanitized.to_owned(),
                Some(UriCredentials {
                    username: "user".to_owned(),
                    password: "secret".to_owned(),
                }),
            )),
            "did not separate {uri:?}"
        );
    }
}

#[test]
fn credential_free_uris_are_accepted() {
    for uri in [
        "smb://host/share",
        "smb://user@host/share",
        "sftp://user@host:2222/path",
        "network:///",
        "sftp://host/share/%FF",
    ] {
        assert_eq!(
            validate_uri_credentials(uri),
            Ok(uri.to_owned()),
            "{uri:?} should be safe"
        );
    }
}

#[test]
fn sanitized_uris_keep_gio_percent_encoding() {
    for (uri, sanitized) in [
        ("trash:///caf%E9.txt", "trash:///caf%E9.txt"),
        ("sftp://host/share/a%2Fb", "sftp://host/share/a%2Fb"),
        ("smb://host/caf%C3%A9", "smb://host/caf%C3%A9"),
        ("smb://host/my share", "smb://host/my%20share"),
    ] {
        assert_eq!(
            sanitize_uri_credentials(uri),
            Ok((sanitized.to_owned(), None)),
            "{uri:?}"
        );
    }
}

#[test]
fn malformed_uris_fail_without_echoing_input() {
    assert_eq!(
        validate_uri_credentials("smb://user%ZZ@host/share"),
        Err(LocationValidationError::InvalidUri)
    );
    assert_eq!(
        LocationValidationError::InvalidUri.to_string(),
        "Enter a valid URI."
    );
}

#[test]
fn backend_unavailable_message_names_the_known_smb_package() {
    let message = backend_unavailable_message("smb://host/share");
    assert!(message.contains("smb://"));
    assert!(message.contains("gvfs-smb"));
}

#[test]
fn backend_unavailable_message_falls_back_for_unknown_schemes() {
    let message = backend_unavailable_message("afp://host/path");
    assert!(message.contains("afp://"));
    assert!(message.contains("distribution"));
}

#[test]
fn backend_unavailable_message_offers_candidate_packages_for_sftp() {
    let message = backend_unavailable_message("sftp://host.example:2222/home/user");
    assert!(message.contains("sftp://"));
    assert!(message.contains("gvfs-backends"));
    assert!(message.contains("distribution"));
    assert!(!message.contains("host.example"));
}

#[test]
fn default_fill_reports_unsupported_synchronously() {
    use super::{
        DirectoryEvent, FileSource, LoadHandle, MetadataOutcome, MetadataRequest, RequestId,
    };
    use crate::model::Location;
    use std::rc::Rc;
    use std::time::Duration;

    struct NoMetadata;
    impl FileSource for NoMetadata {
        fn validate_location(
            &self,
            _location: &Location,
        ) -> Result<(), super::LocationValidationError> {
            Ok(())
        }

        fn enumerate(
            &self,
            _request: super::DirectoryRequest,
            _emit: Rc<dyn Fn(DirectoryEvent)>,
        ) -> LoadHandle {
            LoadHandle::new(|| {})
        }
    }

    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let collected = events.clone();
    let _handle = NoMetadata.fill_metadata(
        MetadataRequest {
            id: RequestId(4),
            entries: Vec::new(),
            full: false,
            include_icon_details: false,
            time_budget: Duration::from_secs(1),
        },
        Rc::new(move |event| collected.borrow_mut().push(event)),
    );
    assert!(matches!(
        events.borrow().as_slice(),
        [DirectoryEvent::MetadataFinished {
            request_id: RequestId(4),
            outcome: MetadataOutcome::Unsupported,
        }]
    ));
}
