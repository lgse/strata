// SPDX-License-Identifier: MIT

use super::super::serialize_pinned_places;

#[test]
fn gtk_bookmark_serialization_sanitizes_uris_with_credentials() {
    let places = vec![
        (
            crate::model::Location::uri("smb://alice@host/safe"),
            "Safe".to_owned(),
        ),
        (
            crate::model::Location::uri("smb://alice:secret@host/private"),
            "Password".to_owned(),
        ),
        (
            crate::model::Location::uri("smb://alice;password=secret@host/private"),
            "Auth".to_owned(),
        ),
        (
            crate::model::Location::uri("smb://alice%3Asecret@host/private"),
            "Encoded".to_owned(),
        ),
    ];

    assert_eq!(
        serialize_pinned_places(&places),
        "smb://alice@host/safe Safe\n\
         smb://alice@host/private Password\n\
         smb://alice@host/private Auth\n\
         smb://alice@host/private Encoded\n"
    );
}
