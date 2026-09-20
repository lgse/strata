// SPDX-License-Identifier: MIT

use super::*;

fn persisted_order(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

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
fn every_reorderable_place_id_is_rendered_by_the_sidebar() {
    for id in STANDARD_PLACE_IDS {
        assert!(
            standard_place(id).is_some() || matches!(*id, "home" | "trash" | "network" | "recent"),
            "{id} must be a standard place or a special destination"
        );
    }
}

#[test]
fn a_persisted_place_order_is_restored_exactly() {
    let saved = [
        "recent",
        "videos",
        "home",
        "downloads",
        "trash",
        "desktop",
        "network",
        "documents",
        "pictures",
    ];
    assert_eq!(resolve_place_order(&persisted_order(&saved)), saved);
}

#[test]
fn unknown_persisted_place_ids_are_dropped() {
    let order = resolve_place_order(&persisted_order(&["desktop", "archive", "videos"]));
    assert_eq!(
        order,
        vec![
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
    );
}

#[test]
fn missing_places_keep_their_default_neighbours() {
    // Orders saved before the special destinations were reorderable list only
    // the XDG ids; Home, Trash, Network and Recent must still lead the sidebar.
    let order = resolve_place_order(&persisted_order(&[
        "desktop",
        "documents",
        "downloads",
        "pictures",
        "videos",
    ]));
    assert_eq!(
        order,
        vec![
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
    );
}

#[test]
fn duplicate_persisted_place_ids_are_deduplicated() {
    let order = resolve_place_order(&persisted_order(&["desktop", "desktop", "videos"]));
    assert_eq!(
        order,
        vec![
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
    );
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
        &BrowserEvent::ColumnsRelocated { from_depth: 1 }
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
    assert!(!sidebar_accepts_file_drop(&Location::uri("recent:///")));
    assert!(!sidebar_accepts_file_drop(&Location::uri(
        "smb://host.example/share"
    )));
}

fn recent_available() -> RecentAvailability {
    RecentAvailability {
        platform_tracking_enabled: true,
        runtime_backend_supported: true,
    }
}

#[test]
fn recent_sidebar_requires_preference_platform_and_backend() {
    assert!(should_show_recent_place(true, recent_available()));
    assert!(!should_show_recent_place(false, recent_available()));
    assert!(!should_show_recent_place(
        true,
        RecentAvailability {
            platform_tracking_enabled: false,
            ..recent_available()
        },
    ));
    assert!(!should_show_recent_place(
        true,
        RecentAvailability {
            runtime_backend_supported: false,
            ..recent_available()
        },
    ));
}
