// SPDX-License-Identifier: MIT

use super::super::places::collect_places;
use super::super::{
    PlaceGroup, RemovableDestination, resolve_place_order, serialize_pinned_places,
};
use crate::model::Location;
use gtk::glib;

#[test]
fn destination_places_follow_sidebar_order_and_skip_unusable_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir()?;
    let home = fixture.path().join("home");
    let documents = home.join("Documents");
    let pictures = home.join("Pictures");
    let pinned = fixture.path().join("work");
    let device = fixture.path().join("usb");
    for directory in [&documents, &pictures, &pinned, &device] {
        std::fs::create_dir_all(directory)?;
    }
    let special_dir = |directory| match directory {
        glib::UserDirectory::Documents => Some(documents.clone()),
        glib::UserDirectory::Downloads => Some(home.join("Downloads")),
        glib::UserDirectory::Pictures => Some(pictures.clone()),
        glib::UserDirectory::Desktop => Some(home.clone()),
        _ => None,
    };

    let places = collect_places(
        &[
            "trash",
            "downloads",
            "documents",
            "pictures",
            "home",
            "desktop",
        ],
        |id| id != "pictures",
        special_dir,
        &home,
        vec![
            (Location::local(&pinned), "Work".to_owned()),
            (Location::uri("sftp://host/share"), "Remote".to_owned()),
            (Location::local(&documents), "Documents again".to_owned()),
        ],
        vec![RemovableDestination {
            id: "volume:usb".to_owned(),
            name: "USB".to_owned(),
            root: device.clone(),
        }],
    );

    let summary: Vec<_> = places
        .iter()
        .map(|place| (place.name.as_str(), place.path.as_path(), place.group))
        .collect();
    assert_eq!(
        summary,
        [
            ("Home", home.as_path(), PlaceGroup::Standard),
            ("Documents", documents.as_path(), PlaceGroup::Standard),
            ("Work", pinned.as_path(), PlaceGroup::Pinned),
            ("USB", device.as_path(), PlaceGroup::Device),
        ],
        "hidden, missing, remote, duplicate, and Desktop-is-Home entries are left out"
    );
    Ok(())
}

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
