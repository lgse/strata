// SPDX-License-Identifier: GPL-3.0-or-later

use super::{backend_unavailable_message, uri_has_embedded_password};

#[test]
fn embedded_passwords_are_detected_in_the_uri_authority() {
    for uri in [
        "smb://user:secret@host/share",
        "sftp://user:secret@host:2222/path",
        "ftp://user:secret@host/public",
    ] {
        assert!(
            uri_has_embedded_password(uri),
            "{uri:?} should be flagged as carrying a password"
        );
    }
}

#[test]
fn uris_without_an_embedded_password_are_not_flagged() {
    for uri in [
        "smb://host/share",
        "smb://user@host/share",
        "sftp://user@host:2222/path",
        "network:///",
        "/regular/absolute/path",
    ] {
        assert!(
            !uri_has_embedded_password(uri),
            "{uri:?} should not be flagged"
        );
    }
}

#[test]
fn backend_unavailable_message_names_the_known_smb_package() {
    let message = backend_unavailable_message("smb://host/share");
    assert!(message.contains("smb://"));
    assert!(message.contains("gvfs-smb"));
}

#[test]
fn backend_unavailable_message_falls_back_for_unknown_schemes() {
    let message = backend_unavailable_message("dav://host/path");
    assert!(message.contains("dav://"));
}
