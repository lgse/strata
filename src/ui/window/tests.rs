// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use super::{
    is_smb_location, is_standard_place_location, parse_pinned_places, remove_pinned_place,
    reorder_places, should_show_standard_place,
};

#[test]
fn places_can_move_before_an_earlier_item() {
    let mut places = vec!["desktop", "documents", "downloads", "pictures", "videos"];

    assert!(reorder_places(&mut places, "videos", "documents", false));
    assert_eq!(
        places,
        vec!["desktop", "videos", "documents", "downloads", "pictures"]
    );
}

#[test]
fn places_can_move_after_a_later_item() {
    let mut places = vec!["desktop", "documents", "downloads", "pictures", "videos"];

    assert!(reorder_places(&mut places, "documents", "pictures", true));
    assert_eq!(
        places,
        vec!["desktop", "downloads", "pictures", "documents", "videos"]
    );
}

#[test]
fn invalid_place_reorders_leave_the_order_unchanged() {
    let original = vec!["desktop", "documents", "downloads"];
    let mut places = original.clone();

    assert!(!reorder_places(&mut places, "missing", "desktop", false));
    assert!(!reorder_places(&mut places, "desktop", "missing", false));
    assert!(!reorder_places(&mut places, "desktop", "desktop", false));
    assert_eq!(places, original);
}

#[test]
fn gtk_bookmarks_become_native_and_remote_pinned_places() {
    let places = parse_pinned_places(
        "file:///home/user/Projects Work\nsftp://host.example/home/user Remote\nfile:///home/user/Projects Duplicate\n",
    );

    assert_eq!(
        places[0].0.native_path(),
        Some(Path::new("/home/user/Projects"))
    );
    assert_eq!(places[0].1, "Work");
    assert_eq!(
        places[1].0.uri_value(),
        Some("sftp://host.example/home/user")
    );
    assert_eq!(places[1].1, "Remote");
    assert_eq!(places.len(), 2);
}

#[test]
fn pinned_places_can_be_removed_by_location() {
    let removed = crate::model::Location::local("/home/user/Removed");
    let retained = crate::model::Location::local("/home/user/Retained");
    let mut places = vec![
        (removed.clone(), "Removed".to_owned()),
        (retained.clone(), "Retained".to_owned()),
    ];

    assert!(remove_pinned_place(&mut places, &removed));
    assert_eq!(places, vec![(retained, "Retained".to_owned())]);
    assert!(!remove_pinned_place(&mut places, &removed));
}

#[test]
fn only_smb_locations_are_disconnectable_network_mounts() {
    assert!(is_smb_location(&crate::model::Location::uri(
        "smb://server/share"
    )));
    assert!(is_smb_location(&crate::model::Location::uri(
        "SMB://server/share"
    )));
    assert!(!is_smb_location(&crate::model::Location::uri(
        "sftp://server/home"
    )));
    assert!(!is_smb_location(&crate::model::Location::local(
        "/mnt/share"
    )));
}

#[test]
fn home_is_already_a_standard_sidebar_location() {
    assert!(is_standard_place_location(&crate::model::Location::local(
        super::home_directory()
    )));
}

#[test]
fn desktop_is_hidden_when_it_points_to_home() {
    let home = Path::new("/home/user");

    assert!(!should_show_standard_place("desktop", home, home));
    assert!(should_show_standard_place(
        "desktop",
        Path::new("/home/user/Desktop"),
        home
    ));
    assert!(should_show_standard_place("documents", home, home));
}
