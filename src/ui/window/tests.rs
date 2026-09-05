// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, path::Path};

use crate::{
    model::Location,
    services::{BuildKind, ReleaseMetadata},
};

use super::{
    MediaRelease, MouseHistoryAction, PinStatus, STANDARD_PLACE_IDS,
    accepts_sidebar_reorder_payload, begin_media_release, is_open_terminal_shortcut,
    is_sidebar_focus_shortcut, is_smb_location, is_standard_place_location,
    is_toggle_hidden_shortcut, is_undo_shortcut, jump_direction, media_release_label,
    mount_release_action, mouse_history_action, page_direction, parse_pinned_drag_source,
    parse_pinned_places, pin_status, remove_pinned_place, reorder_pinned_places, reorder_places,
    resolve_place_order, serialize_pinned_places, should_show_standard_place,
    sidebar_accepts_file_drop, sidebar_update_label, standard_place, type_to_search_query,
    vim_focus_direction, volume_release_action,
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
fn plain_single_pane_arrows_move_focus_not_directories() {
    use super::{BrowserMode, SinglePaneArrow, single_pane_arrow_action};
    use gtk::gdk::{Key, ModifierType};
    let plain = ModifierType::empty();
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Grid, Key::Left, plain, false, true),
        Some(SinglePaneArrow::Native)
    );
    for mode in [BrowserMode::Grid, BrowserMode::Explorer] {
        assert_eq!(
            single_pane_arrow_action(mode, Key::Left, plain, true, true),
            Some(SinglePaneArrow::Sidebar)
        );
        for key in [Key::Up, Key::Down] {
            assert_eq!(
                single_pane_arrow_action(mode, key, plain, true, true),
                Some(SinglePaneArrow::Native)
            );
        }
        for key in [Key::Left, Key::Right, Key::Up] {
            assert_eq!(
                single_pane_arrow_action(mode, key, ModifierType::ALT_MASK, true, true),
                None
            );
        }
        assert_eq!(
            single_pane_arrow_action(mode, Key::Return, plain, true, true),
            None
        );
    }
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Explorer, Key::Right, plain, true, true),
        Some(SinglePaneArrow::Stay)
    );
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Explorer, Key::Left, plain, true, false),
        Some(SinglePaneArrow::Stay)
    );
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Grid, Key::Left, plain, true, false),
        Some(SinglePaneArrow::Native)
    );
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Columns, Key::Left, plain, true, true),
        None
    );
    for modifier in [ModifierType::SHIFT_MASK, ModifierType::CONTROL_MASK] {
        assert_eq!(
            single_pane_arrow_action(BrowserMode::Grid, Key::Left, modifier, true, true),
            Some(SinglePaneArrow::Native)
        );
    }
}

#[test]
fn sidebar_arrows_and_vim_keys_share_focus_directions() {
    use gtk::gdk::Key;
    for (arrow, vim) in [
        (Key::Left, Key::h),
        (Key::Right, Key::l),
        (Key::Up, Key::k),
        (Key::Down, Key::j),
    ] {
        assert_eq!(
            super::sidebar_focus_direction(arrow),
            vim_focus_direction(vim)
        );
    }
    assert_eq!(super::sidebar_focus_direction(Key::Return), None);
}

#[test]
fn navigation_keys_claim_keyboard_ownership_but_commands_do_not() {
    use gtk::gdk::{Key, ModifierType};
    for key in [
        Key::Up,
        Key::Down,
        Key::h,
        Key::j,
        Key::k,
        Key::l,
        Key::Tab,
        Key::ISO_Left_Tab,
        Key::Page_Down,
        Key::Return,
    ] {
        assert!(super::is_browser_navigation_key(key, ModifierType::empty()));
    }
    assert!(super::is_browser_navigation_key(
        Key::Down,
        ModifierType::SHIFT_MASK
    ));
    assert!(super::is_browser_navigation_key(
        Key::Left,
        ModifierType::ALT_MASK
    ));
    for key in [Key::v, Key::c, Key::x, Key::z, Key::Control_L, Key::Delete] {
        assert!(!super::is_browser_navigation_key(
            key,
            ModifierType::CONTROL_MASK
        ));
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
fn undo_shortcut_requires_control_without_shift_or_alt() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_undo_shortcut(gtk::gdk::Key::z, control));
    assert!(is_undo_shortcut(gtk::gdk::Key::Z, control));
    assert!(!is_undo_shortcut(
        gtk::gdk::Key::z,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_undo_shortcut(gtk::gdk::Key::z, control | shift));
    assert!(!is_undo_shortcut(gtk::gdk::Key::z, control | alt));
}

#[test]
fn page_keys_map_to_a_scroll_direction() {
    assert_eq!(page_direction(gtk::gdk::Key::Page_Up), Some(-1));
    assert_eq!(page_direction(gtk::gdk::Key::KP_Page_Up), Some(-1));
    assert_eq!(page_direction(gtk::gdk::Key::Page_Down), Some(1));
    assert_eq!(page_direction(gtk::gdk::Key::KP_Page_Down), Some(1));
    assert_eq!(page_direction(gtk::gdk::Key::Home), None);
}

#[test]
fn jump_shortcut_requires_control_without_other_command_modifiers() {
    use gtk::gdk::{Key, ModifierType};
    let control = ModifierType::CONTROL_MASK;

    assert_eq!(jump_direction(Key::Up, control), Some(-1));
    assert_eq!(jump_direction(Key::Down, control), Some(1));
    assert_eq!(jump_direction(Key::Left, control), None);
    assert_eq!(jump_direction(Key::Up, ModifierType::empty()), None);
    for modifier in [
        ModifierType::SHIFT_MASK,
        ModifierType::ALT_MASK,
        ModifierType::SUPER_MASK,
    ] {
        assert_eq!(jump_direction(Key::Up, control | modifier), None);
    }
}

#[test]
fn toggle_hidden_shortcut_accepts_h_or_period_with_only_control() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_toggle_hidden_shortcut(gtk::gdk::Key::h, control));
    assert!(is_toggle_hidden_shortcut(gtk::gdk::Key::H, control));
    assert!(is_toggle_hidden_shortcut(gtk::gdk::Key::period, control));
    assert!(!is_toggle_hidden_shortcut(
        gtk::gdk::Key::h,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_toggle_hidden_shortcut(
        gtk::gdk::Key::h,
        control | shift
    ));
    assert!(!is_toggle_hidden_shortcut(gtk::gdk::Key::h, control | alt));
    assert!(!is_toggle_hidden_shortcut(
        gtk::gdk::Key::period,
        control | shift
    ));
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
fn type_to_search_accepts_printable_keys_without_command_modifiers() {
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::a, gtk::gdk::ModifierType::empty()),
        Some('a')
    );
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::A, gtk::gdk::ModifierType::SHIFT_MASK),
        Some('A')
    );
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::space, gtk::gdk::ModifierType::empty()),
        Some(' ')
    );
}

#[test]
fn type_to_search_ignores_shortcuts_and_non_printable_keys() {
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::k, gtk::gdk::ModifierType::CONTROL_MASK),
        None
    );
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::F5, gtk::gdk::ModifierType::empty()),
        None
    );
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

fn persisted_order(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn every_reorderable_place_id_is_a_known_standard_place() {
    for id in STANDARD_PLACE_IDS {
        assert!(
            standard_place(id).is_some(),
            "{id} must be a standard place"
        );
    }
}

#[test]
fn a_persisted_place_order_is_restored_exactly() {
    let order = resolve_place_order(&persisted_order(&["videos", "downloads", "desktop"]));
    assert_eq!(
        order,
        vec!["videos", "downloads", "desktop", "documents", "pictures"]
    );
}

#[test]
fn unknown_persisted_place_ids_are_dropped() {
    let order = resolve_place_order(&persisted_order(&["desktop", "archive", "videos"]));
    assert_eq!(
        order,
        vec!["desktop", "videos", "documents", "downloads", "pictures"]
    );
}

#[test]
fn missing_places_are_appended_in_default_order() {
    let order = resolve_place_order(&persisted_order(&["pictures"]));
    assert_eq!(
        order,
        vec!["pictures", "desktop", "documents", "downloads", "videos"]
    );
}

#[test]
fn duplicate_persisted_place_ids_are_deduplicated() {
    let order = resolve_place_order(&persisted_order(&["desktop", "desktop", "videos"]));
    assert_eq!(
        order,
        vec!["desktop", "videos", "documents", "downloads", "pictures"]
    );
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

fn pinned_names(places: &[(crate::model::Location, String)]) -> Vec<&str> {
    places.iter().map(|(_, name)| name.as_str()).collect()
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

#[test]
fn sidebar_sync_runs_only_for_location_changes() {
    use super::SidebarState;
    use crate::app::BrowserEvent;
    use crate::model::Location;

    assert!(SidebarState::event_changes_active_place(
        &BrowserEvent::Reset
    ));
    assert!(SidebarState::event_changes_active_place(
        &BrowserEvent::ColumnAdded {
            depth: 1,
            location: Location::local("/fixture/sub"),
        }
    ));
    assert!(SidebarState::event_changes_active_place(
        &BrowserEvent::ColumnsTruncated { len: 1 }
    ));
    assert!(SidebarState::event_changes_active_place(
        &BrowserEvent::FocusChanged {
            depth: 0,
            position: Some(2),
        }
    ));
    assert!(!SidebarState::event_changes_active_place(
        &BrowserEvent::EntriesInserted {
            depth: 0,
            insertions: Vec::new(),
        }
    ));
    assert!(!SidebarState::event_changes_active_place(
        &BrowserEvent::MetadataFilled {
            depth: 0,
            updates: Vec::new(),
        }
    ));
    assert!(!SidebarState::event_changes_active_place(
        &BrowserEvent::SortingStarted { depth: 0 }
    ));
    assert!(!SidebarState::event_changes_active_place(
        &BrowserEvent::SortingFinished { depth: 0 }
    ));
    assert!(!SidebarState::event_changes_active_place(
        &BrowserEvent::LoadFinished {
            depth: 0,
            truncated: false,
        }
    ));
    assert!(!SidebarState::event_changes_active_place(
        &BrowserEvent::TransferCompleted
    ));
}

#[test]
fn volume_release_prefers_eject_and_hides_fixed_disks() {
    assert_eq!(
        volume_release_action(true, false, false),
        Some(MediaRelease::EjectVolume)
    );
    assert_eq!(
        volume_release_action(true, true, true),
        Some(MediaRelease::EjectVolume)
    );
    assert_eq!(
        volume_release_action(false, true, false),
        Some(MediaRelease::EjectMount)
    );
    assert_eq!(
        volume_release_action(false, true, true),
        Some(MediaRelease::EjectMount)
    );
    assert_eq!(
        volume_release_action(false, false, true),
        Some(MediaRelease::UnmountMount)
    );
    assert_eq!(volume_release_action(false, false, false), None);
}

#[test]
fn mount_release_prefers_eject_and_hides_fixed_disks() {
    assert_eq!(
        mount_release_action(true, false),
        Some(MediaRelease::EjectMount)
    );
    assert_eq!(
        mount_release_action(true, true),
        Some(MediaRelease::EjectMount)
    );
    assert_eq!(
        mount_release_action(false, true),
        Some(MediaRelease::UnmountMount)
    );
    assert_eq!(mount_release_action(false, false), None);
}

#[test]
fn media_release_labels_match_nautilus_wording() {
    assert_eq!(media_release_label(MediaRelease::EjectVolume), "Eject");
    assert_eq!(media_release_label(MediaRelease::EjectMount), "Eject");
    assert_eq!(media_release_label(MediaRelease::UnmountMount), "Unmount");
}

#[test]
fn media_release_guard_rejects_repeated_actions_until_completion() {
    let in_flight = Cell::new(false);

    assert!(begin_media_release(&in_flight));
    assert!(!begin_media_release(&in_flight));

    in_flight.set(false);
    assert!(begin_media_release(&in_flight));
}

#[test]
fn file_payloads_are_not_claimed_as_sidebar_reorders() {
    assert!(accepts_sidebar_reorder_payload(true, false));
    assert!(!accepts_sidebar_reorder_payload(true, true));
    assert!(!accepts_sidebar_reorder_payload(false, true));
}

#[test]
fn sidebar_file_drops_accept_local_places_but_not_virtual_locations() {
    assert!(sidebar_accepts_file_drop(&Location::local(
        "/run/media/user/stick"
    )));
    assert!(sidebar_accepts_file_drop(&Location::local(
        "/home/user/Documents"
    )));
    assert!(!sidebar_accepts_file_drop(&Location::uri("trash:///")));
    assert!(!sidebar_accepts_file_drop(&Location::uri("network:///")));
    assert!(!sidebar_accepts_file_drop(&Location::uri(
        "smb://host.example/share"
    )));
}
