// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use crate::services::{BuildKind, ReleaseMetadata};

use super::{
    MouseHistoryAction, PinStatus, is_native_editing_shortcut, is_open_terminal_shortcut,
    is_sidebar_focus_shortcut, is_smb_location, is_standard_place_location, mouse_history_action,
    parse_pinned_places, pin_status, remove_pinned_place, reorder_places, serialize_pinned_places,
    should_show_standard_place, sidebar_update_label, vim_focus_direction,
};

fn release(version: &str, kind: BuildKind) -> ReleaseMetadata {
    ReleaseMetadata {
        version: version.to_owned(),
        url: "https://example.test/release".to_owned(),
        notes: String::new(),
        note_blocks: Vec::new(),
        kind,
        tag: format!("v{version}"),
        published_at: None,
        commit: None,
    }
}

#[test]
fn sidebar_update_label_stays_plain_for_a_stable_release() {
    assert_eq!(
        sidebar_update_label(&release("0.6.0", BuildKind::Stable)),
        "v0.6.0 available"
    );
}

#[test]
fn sidebar_update_label_names_the_build_kind_for_a_prerelease() {
    assert_eq!(
        sidebar_update_label(&release("0.6.0-rc.1", BuildKind::Rc)),
        "v0.6.0-rc.1 (Release candidate) available"
    );
    assert_eq!(
        sidebar_update_label(&release("0.6.0-nightly.20260901", BuildKind::Nightly)),
        "v0.6.0-nightly.20260901 (Nightly) available"
    );
}

#[test]
fn mouse_history_buttons_map_to_navigation_actions() {
    assert_eq!(mouse_history_action(8), Some(MouseHistoryAction::Back));
    assert_eq!(mouse_history_action(9), Some(MouseHistoryAction::Forward));
    for button in [1, 2, 3, 4, 5, 6, 7, 10] {
        assert_eq!(mouse_history_action(button), None);
    }
}

#[test]
fn open_terminal_shortcut_requires_only_control() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_open_terminal_shortcut(gtk::gdk::Key::t, control));
    assert!(is_open_terminal_shortcut(gtk::gdk::Key::T, control));
    assert!(!is_open_terminal_shortcut(
        gtk::gdk::Key::t,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_open_terminal_shortcut(
        gtk::gdk::Key::t,
        control | shift
    ));
    assert!(!is_open_terminal_shortcut(gtk::gdk::Key::t, control | alt));
    assert!(!is_open_terminal_shortcut(gtk::gdk::Key::F4, control));
}

#[test]
fn sidebar_focus_shortcut_requires_control_and_shift() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;

    assert!(is_sidebar_focus_shortcut(gtk::gdk::Key::b, control | shift));
    assert!(is_sidebar_focus_shortcut(gtk::gdk::Key::B, control | shift));
    assert!(!is_sidebar_focus_shortcut(gtk::gdk::Key::b, control));
}

#[test]
fn native_editing_shortcuts_are_left_to_the_focused_widget() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;

    for key in [
        gtk::gdk::Key::a,
        gtk::gdk::Key::c,
        gtk::gdk::Key::v,
        gtk::gdk::Key::x,
    ] {
        assert!(is_native_editing_shortcut(key, control));
    }
    assert!(!is_native_editing_shortcut(
        gtk::gdk::Key::c,
        control | gtk::gdk::ModifierType::SHIFT_MASK
    ));
}

#[test]
fn vim_focus_keys_map_to_gtk_directions() {
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::h),
        Some(gtk::DirectionType::Left)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::j),
        Some(gtk::DirectionType::Down)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::k),
        Some(gtk::DirectionType::Up)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::l),
        Some(gtk::DirectionType::Right)
    );
    assert_eq!(vim_focus_direction(gtk::gdk::Key::Down), None);
}

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
fn gtk_bookmarks_sanitize_uris_with_credentials() {
    let places = parse_pinned_places(
        "smb://alice@host/safe Safe\nsmb://alice:secret@host/private Password\nsmb://alice%3Asecret@host/private Encoded password delimiter\nsmb://alice;password=secret@host/private Auth\nsmb://alice%3Bpassword=secret@host/private Encoded auth delimiter\nsmb://alice;password=sec%72et@host/private Encoded value\nsmb://alice%ZZ@host/private Invalid\n",
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
