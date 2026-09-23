// SPDX-License-Identifier: MIT

use super::*;

fn pinned_names(places: &[(crate::model::Location, String)]) -> Vec<&str> {
    places.iter().map(|(_, name)| name.as_str()).collect()
}

fn pinned(paths: &[&str]) -> Vec<(crate::model::Location, String)> {
    paths
        .iter()
        .map(|path| {
            (
                crate::model::Location::local(format!("/home/user/{path}")),
                (*path).to_owned(),
            )
        })
        .collect()
}

#[test]
fn pinned_places_can_move_before_an_earlier_place() {
    let mut places = pinned(&["Projects", "Notes", "Archive"]);

    assert!(reorder_pinned_places(&mut places, 2, 0, false));
    assert_eq!(pinned_names(&places), ["Archive", "Projects", "Notes"]);
}

#[test]
fn pinned_places_can_move_after_a_later_place() {
    let mut places = pinned(&["Projects", "Notes", "Archive"]);

    assert!(reorder_pinned_places(&mut places, 0, 2, true));
    assert_eq!(pinned_names(&places), ["Notes", "Archive", "Projects"]);
}

#[test]
fn pinned_reorders_that_keep_the_position_are_ignored() {
    let original = pinned(&["Projects", "Notes", "Archive"]);
    let mut places = original.clone();

    assert!(!reorder_pinned_places(&mut places, 1, 1, false));
    assert!(!reorder_pinned_places(&mut places, 1, 0, true));
    assert!(!reorder_pinned_places(&mut places, 1, 2, false));
    assert_eq!(places, original);
}

#[test]
fn out_of_range_pinned_reorders_leave_the_order_unchanged() {
    let original = pinned(&["Projects", "Notes"]);
    let mut places = original.clone();

    assert!(!reorder_pinned_places(&mut places, 5, 0, false));
    assert!(!reorder_pinned_places(&mut places, 0, 5, true));
    assert_eq!(places, original);
}

#[test]
fn pinned_reorders_leave_the_other_places_untouched() {
    let mut places = pinned(&["Documents", "Projects", "Notes"]);

    assert!(reorder_pinned_places(&mut places, 2, 1, false));
    assert_eq!(pinned_names(&places), ["Documents", "Notes", "Projects"]);
}

#[test]
fn only_pinned_drag_payloads_resolve_to_a_pinned_place() {
    assert_eq!(parse_pinned_drag_source("pinned:3"), Some(3));
    assert_eq!(parse_pinned_drag_source("documents"), None);
    assert_eq!(parse_pinned_drag_source("pinned:documents"), None);
}

#[test]
fn gtk_bookmarks_become_native_and_remote_pinned_places() {
    let places = parse_pinned_places(
        b"file:///home/user/Projects Work\nsftp://host.example/home/user Remote\nfile:///home/user/Projects Duplicate\n",
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
fn gtk_bookmarks_survive_non_utf8_labels_and_windows_line_endings() {
    let places = parse_pinned_places(b"file:///tmp/a A\r\nfile:///tmp/b \xff\nfile:///tmp/c C\n");

    assert_eq!(places.len(), 3);
    assert_eq!(places[0].1, "A");
    assert_eq!(places[1].1, "\u{FFFD}");
    assert_eq!(places[2].1, "C");
}

#[test]
fn gtk_bookmarks_drop_lines_with_non_utf8_uris() {
    let places = parse_pinned_places(b"file:///tmp/\xff bad\nfile:///tmp/good Good\n");

    assert_eq!(places.len(), 1);
    assert_eq!(places[0].0.native_path(), Some(Path::new("/tmp/good")));
    assert_eq!(places[0].1, "Good");
}

#[test]
fn gtk_bookmarks_sanitize_uris_with_credentials() {
    let places = parse_pinned_places(
        b"smb://alice@host/safe Safe\nsmb://alice:secret@host/private Password\nsmb://alice%3Asecret@host/private Encoded password delimiter\nsmb://alice;password=secret@host/private Auth\nsmb://alice%3Bpassword=secret@host/private Encoded auth delimiter\nsmb://alice;password=sec%72et@host/private Encoded value\nsmb://alice%ZZ@host/private Invalid\n",
    );

    assert_eq!(places.len(), 2);
    assert_eq!(
        places[0]
            .0
            .uri_value()
            .expect("remote place should have a URI")
            .trim_end_matches('/'),
        "smb://alice@host/safe"
    );
    assert_eq!(
        places[1]
            .0
            .uri_value()
            .expect("remote place should have a URI")
            .trim_end_matches('/'),
        "smb://alice@host/private"
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

#[test]
fn pin_status_distinguishes_available_pinned_and_standard_locations() {
    let pinned = crate::model::Location::uri("smb://server/share/folder");
    let places = vec![(pinned.clone(), "Folder".to_owned())];

    assert_eq!(pin_status(&places, &pinned), PinStatus::Pinned);
    assert_eq!(
        pin_status(
            &places,
            &crate::model::Location::uri("smb://server/share/other")
        ),
        PinStatus::Available
    );
    assert_eq!(
        pin_status(
            &places,
            &crate::model::Location::local(super::home_directory())
        ),
        PinStatus::Unavailable
    );
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
fn pinned_place_changes_merge_with_the_shared_bookmarks_file() {
    gtk_test(
        "ui::window::tests::bookmarks::pinned_place_changes_merge_with_the_shared_bookmarks_file",
        || {
            let first = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            let existing = Location::local("/tmp/existing");
            first
                .state
                .pin_location(existing.clone(), "Existing".into());
            let second = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            let pinned = Location::local("/tmp/pinned");
            first.state.pin_location(pinned.clone(), "Pinned".into());
            second.state.unpin_location(&existing);
            assert_eq!(
                load_pinned_places().expect("merged pins"),
                vec![(pinned, "Pinned".into())]
            );

            second.state.pin_location(existing, "Existing".into());
            let path = pinned_places_path();
            std::fs::write(&path, "file:///tmp/external External\nfile:///tmp/pinned Renamed\nfile:///tmp/existing Existing\n")
                .expect("external edit");
            second.state.reorder_pinned_place(1, 0, false);
            assert_eq!(
                std::fs::read_to_string(&path).expect("reordered pins"),
                "file:///tmp/external External\nfile:///tmp/existing Existing\nfile:///tmp/pinned Renamed\n"
            );
            std::fs::write(
                &path,
                "file:///tmp/external External\nfile:///tmp/pinned Renamed\n",
            )
            .expect("external removal");
            second.state.reorder_pinned_place(1, 2, true);
            let saved = load_pinned_places().expect("missing source is not resurrected");
            assert_eq!(saved.len(), 2);
            assert_eq!(*second.state.pinned_places.borrow(), saved);
            first.disconnect();
            second.disconnect();
        },
    );
}

#[test]
fn pinning_with_a_non_utf8_label_preserves_shared_bookmarks() {
    gtk_test(
        "ui::window::tests::bookmarks::pinning_with_a_non_utf8_label_preserves_shared_bookmarks",
        || {
            let path = pinned_places_path();
            std::fs::create_dir_all(path.parent().expect("bookmarks parent"))
                .expect("create bookmarks parent");
            std::fs::write(
                &path,
                b"file:///fixtures/existing Existing\nfile:///fixtures/lossy \xff\n",
            )
            .expect("seed non-UTF-8 bookmark label");

            let first = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            let second = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            let initial = vec![
                (Location::local("/fixtures/existing"), "Existing".into()),
                (Location::local("/fixtures/lossy"), "\u{FFFD}".into()),
            ];
            assert_eq!(*first.state.pinned_places.borrow(), initial);
            assert_eq!(*second.state.pinned_places.borrow(), initial);

            first
                .state
                .pin_location(Location::local("/fixtures/first"), "First".into());
            second
                .state
                .pin_location(Location::local("/fixtures/second"), "Second".into());
            assert_eq!(
                load_pinned_places().expect("saved bookmarks"),
                vec![
                    (Location::local("/fixtures/existing"), "Existing".into()),
                    (Location::local("/fixtures/lossy"), "\u{FFFD}".into()),
                    (Location::local("/fixtures/first"), "First".into()),
                    (Location::local("/fixtures/second"), "Second".into()),
                ]
            );
            std::fs::read_to_string(path).expect("saved bookmarks are valid UTF-8");
            first.disconnect();
            second.disconnect();
        },
    );
}

#[test]
fn failed_bookmark_reads_and_saves_preserve_disk_and_window_state() {
    gtk_test(
        "ui::window::tests::bookmarks::failed_bookmark_reads_and_saves_preserve_disk_and_window_state",
        || {
            let sidebar = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            let existing = Location::local("/tmp/existing");
            sidebar
                .state
                .pin_location(existing.clone(), "Existing".into());
            let original = sidebar.state.pinned_places.borrow().clone();
            let path = pinned_places_path();
            std::fs::remove_file(&path).expect("remove seeded bookmarks file");
            std::fs::create_dir(&path).expect("unreadable bookmarks directory");
            sidebar
                .state
                .pin_location(Location::local("/tmp/new"), "New".into());
            assert!(path.is_dir());
            assert_eq!(*sidebar.state.pinned_places.borrow(), original);

            let contents = serialize_pinned_places(&original);
            std::fs::remove_dir(&path).expect("remove directory fixture");
            std::fs::write(&path, &contents).expect("restore readable bookmarks");
            let target = path.with_extension("target");
            std::fs::rename(&path, &target).expect("move fixture");
            std::os::unix::fs::symlink(&target, &path).expect("readable but non-replaceable file");
            sidebar.state.unpin_location(&existing);
            assert_eq!(
                std::fs::read_to_string(&target).expect("preserved target"),
                contents
            );
            assert!(path.is_symlink());
            assert_eq!(*sidebar.state.pinned_places.borrow(), original);
            sidebar.disconnect();
        },
    );
}
