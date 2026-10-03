// SPDX-License-Identifier: MIT

use super::super::{resolve_place_order, serialize_pinned_places};

#[test]
fn existing_sidebar_order_gains_music_after_downloads() {
    let previous_order = [
        "home",
        "trash",
        "network",
        "recent",
        "desktop",
        "documents",
        "downloads",
        "pictures",
        "videos",
    ]
    .map(str::to_owned);
    let upgraded = resolve_place_order(&previous_order);
    assert_eq!(
        upgraded,
        [
            "home",
            "trash",
            "network",
            "recent",
            "desktop",
            "documents",
            "downloads",
            "music",
            "pictures",
            "videos"
        ]
    );
}

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
