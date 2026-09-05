// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn breadcrumbs_preserve_each_native_ancestor() {
    let location = Location::local("/home/user/project");

    assert_eq!(
        location.breadcrumbs(),
        vec![
            Location::local("/"),
            Location::local("/home"),
            Location::local("/home/user"),
            Location::local("/home/user/project"),
        ]
    );
}

#[test]
fn uri_locations_remain_explicit_and_have_one_breadcrumb() {
    let trash = Location::uri("trash:///");

    assert_eq!(trash.native_path(), None);
    assert_eq!(trash.uri_value(), Some("trash:///"));
    assert_eq!(trash.display_name(), "Trash");
    assert_eq!(trash.display_path(), "trash:///");
    assert_eq!(trash.breadcrumbs(), vec![trash.clone()]);
    assert_eq!(trash.parent(), None);
}

#[test]
fn remote_locations_keep_uri_parents_and_breadcrumbs() {
    let location = Location::uri("smb://server/share/folder");

    assert_eq!(location.parent(), Some(Location::uri("smb://server/share")));
    let breadcrumbs = location.breadcrumbs();
    assert_eq!(breadcrumbs.last(), Some(&location));
    assert!(breadcrumbs.contains(&Location::uri("smb://server/share")));
    assert!(
        breadcrumbs
            .iter()
            .all(|crumb| crumb.native_path().is_none())
    );
}

#[test]
fn backend_names_distinguish_native_remote_and_malformed_locations() {
    assert_eq!(
        Location::local("/home/alice/private").backend_name(),
        "native"
    );
    assert_eq!(
        Location::uri("sftp://example.com/private").backend_name(),
        "sftp"
    );
    assert_eq!(Location::uri("not a uri").backend_name(), "uri");
}

#[test]
fn diagnostic_paths_preserve_native_paths_and_redact_remote_secrets() {
    assert_eq!(
        Location::local("/home/alice/private").diagnostic_path(),
        "/home/alice/private"
    );
    assert_eq!(
        Location::uri(
            "sftp://alice:password;key=secret@example.com/home/alice?token=secret#private-fragment"
        )
        .diagnostic_path(),
        "sftp://example.com/home/alice"
    );
    assert_eq!(
        Location::uri("not a uri with alice:password@example.com").diagnostic_path(),
        "<invalid-uri>"
    );
}

#[test]
fn display_paths_hide_remote_credentials_but_preserve_usernames() {
    for uri in [
        "smb://alice:secret@host/share",
        "smb://alice%3Asecret@host/share",
        "smb://alice:sec%72et@host/share",
        "smb://alice;password=secret@host/share",
        "smb://alice%3Bpassword=secret@host/share",
        "smb://alice;password=sec%72et@host/share",
    ] {
        assert_eq!(Location::uri(uri).display_path(), "smb://host/share");
    }
    assert_eq!(
        Location::uri("smb://alice@host/share").display_path(),
        "smb://alice@host/share"
    );
    assert_eq!(
        Location::uri("smb://alice%ZZ@host/share").display_path(),
        "<invalid-uri>"
    );
}

#[test]
fn root_has_one_breadcrumb() {
    assert_eq!(
        Location::local("/").breadcrumbs(),
        vec![Location::local("/")]
    );
}

#[test]
fn folder_colors_parse_names_and_resolve_hex() {
    assert_eq!(FolderColor::from_name("red"), Some(FolderColor::Red));
    assert_eq!(FolderColor::from_name("Blue"), Some(FolderColor::Blue));
    assert_eq!(FolderColor::from_name("green"), Some(FolderColor::Green));
    assert_eq!(FolderColor::from_name("grey"), Some(FolderColor::Gray));
    assert_eq!(FolderColor::from_name("unknown"), None);
    assert_eq!(FolderColor::Red.hex(), "#e5484d");
    assert_eq!(FolderColor::Blue.hex(), "#0090ff");
    assert_eq!(FolderColor::ALL.len(), 7);
}

#[test]
fn folder_color_values_parse_and_resolve_hex() {
    assert_eq!(
        FolderColorValue::parse("red"),
        Some(FolderColorValue::Preset(FolderColor::Red))
    );
    assert_eq!(
        FolderColorValue::parse("#34d399"),
        Some(FolderColorValue::Custom("#34d399".to_owned()))
    );
    assert_eq!(
        FolderColorValue::parse("#FFF"),
        Some(FolderColorValue::Custom("#fff".to_owned()))
    );
    assert_eq!(FolderColorValue::parse("not-a-color"), None);
    assert_eq!(FolderColorValue::parse("#invalid"), None);
    assert_eq!(FolderColorValue::Preset(FolderColor::Red).hex(), "#e5484d");
    assert_eq!(
        FolderColorValue::Custom("#34d399".to_owned()).hex(),
        "#34d399"
    );
    assert_eq!(
        FolderColorValue::Preset(FolderColor::Red).to_preference_string(),
        "red"
    );
    assert_eq!(
        FolderColorValue::Custom("#34d399".to_owned()).to_preference_string(),
        "#34d399"
    );
}

#[test]
fn transfer_targets_keep_the_item_name_under_the_destination() {
    assert_eq!(
        Location::local("/home/user/report.txt")
            .transfer_target(&Location::local("/home/user/archive")),
        Some(Location::local("/home/user/archive/report.txt"))
    );
    assert_eq!(
        Location::uri("smb://host/share/notes/report.txt")
            .transfer_target(&Location::uri("smb://host/share/archive")),
        Some(Location::uri("smb://host/share/archive/report.txt"))
    );
    assert_eq!(
        Location::local("/home/user/a b.txt").transfer_target(&Location::uri("smb://host/share")),
        Some(Location::uri("smb://host/share/a%20b.txt"))
    );
}

#[test]
fn children_reject_names_that_would_escape_the_parent() {
    let parent = Location::local("/home/user");

    for name in ["", ".", "..", "nested/child"] {
        assert_eq!(parent.child(std::ffi::OsStr::new(name)), None);
    }
    assert_eq!(
        Location::local("/").transfer_target(&parent),
        None,
        "a root has no name to carry into a destination"
    );
}
