// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    LocationValidationError, UriCredentials, backend_unavailable_message, sanitize_uri_credentials,
    validate_uri_credentials,
};

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
    for uri in [
        "smb://user:secret@host/share",
        "smb://user%3Asecret@host/share",
        "smb://user;password=secret@host/share",
        "smb://user%3Bpassword=secret@host/share",
    ] {
        assert_eq!(
            sanitize_uri_credentials(uri),
            Ok((
                "smb://user@host/share".to_owned(),
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
    ] {
        assert_eq!(
            validate_uri_credentials(uri),
            Ok(()),
            "{uri:?} should be safe"
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
    assert!(!message.contains("host.example"));
}

#[test]
fn backend_unavailable_message_never_claims_one_universal_package() {
    for uri in ["smb://host/share", "sftp://host/path", "dav://host/path"] {
        let message = backend_unavailable_message(uri);
        assert!(
            message.contains("distribution"),
            "{uri} names a package as if every distribution used it: {message}"
        );
    }
}
