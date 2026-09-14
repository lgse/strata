// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn only_camera_mount_roots_present_as_flat_photo_libraries() {
    for uri in ["gphoto2://camera/", "gphoto2://Apple_Inc._iPhone_fixture/"] {
        let root = Location::uri(uri);
        assert!(root.is_camera_photo_root());
        assert_eq!(root.display_name(), "Photos");
        assert_eq!(root.uri_value(), Some(uri));
        let child = root
            .child(std::ffi::OsStr::new("202409_a"))
            .expect("child folder");
        assert!(!child.is_camera_photo_root());
        assert_eq!(child.display_name(), "202409_a");
        assert!(root.contains_camera_photo_location(&child));
        assert!(root.contains_camera_photo_location(
            &child.child(std::ffi::OsStr::new("IMG.JPG")).expect("photo")
        ));
        assert!(
            !root.contains_camera_photo_location(&Location::uri(
                "gphoto2://different-device/IMG.JPG"
            ))
        );
    }
    for uri in [
        "mtp://phone/",
        "afc://phone/",
        "sftp://server/",
        "file:///",
        "trash:///",
    ] {
        assert!(!Location::uri(uri).is_camera_photo_root());
    }
    assert!(!Location::local("/camera").is_camera_photo_root());
}

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
fn uri_display_names_are_percent_decoded() {
    assert_eq!(
        Location::uri("smb://server/share/My%20Share").display_name(),
        "My Share"
    );
    assert_eq!(
        Location::uri("sftp://host/caf%C3%A9/").display_name(),
        "café"
    );
    assert_eq!(Location::uri("sftp://host/").display_name(), "host");
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
fn rebasing_preserves_uri_descendants_and_rejects_unrelated_or_mixed_locations() {
    for scheme in ["smb", "sftp"] {
        let from = Location::uri(format!("{scheme}://host/share/old"));
        let to = Location::uri(format!("{scheme}://host/share/new%20folder"));
        for suffix in ["", "/nested/report%20%231.txt"] {
            let original = Location::uri(format!("{scheme}://host/share/old{suffix}"));
            let rebased = original.rebase(&from, &to).expect("rebased URI");
            let expected =
                gio::File::for_uri(&format!("{scheme}://host/share/new%20folder{suffix}"));
            assert!(gio::File::for_uri(rebased.uri_value().expect("rebased URI")).equal(&expected));
            assert!(rebased.native_path().is_none());
        }
        for unrelated in [
            format!("{scheme}://host/share/older/nested"),
            format!("{scheme}://other/share/old/nested"),
            format!("{scheme}://host/elsewhere/old"),
        ] {
            assert!(Location::uri(unrelated).rebase(&from, &to).is_none());
        }
        assert!(
            from.rebase(&from, &Location::local("/fixture/new"))
                .is_none()
        );
    }
    let from = Location::local("/fixture/old");
    let to = Location::local("/fixture/new");
    assert_eq!(from.rebase(&from, &to), Some(to.clone()));
    assert_eq!(
        Location::local("/fixture/old/nested").rebase(&from, &to),
        Some(Location::local("/fixture/new/nested"))
    );
    assert!(
        Location::local("/fixture/older")
            .rebase(&from, &to)
            .is_none()
    );
}

#[test]
fn is_within_matches_native_descendants() {
    let child = Location::local("/home/user/project/src");
    let parent = Location::local("/home/user/project");
    let unrelated = Location::local("/home/other");

    assert!(child.is_within(&parent));
    assert!(!parent.is_within(&child));
    assert!(!child.is_within(&unrelated));
}

#[test]
fn is_within_matches_remote_descendants() {
    let child = Location::uri("sftp://user@host/mnt/share/folder");
    let parent = Location::uri("sftp://user@host/mnt/share");
    let unrelated = Location::uri("sftp://user@host/other");

    assert!(child.is_within(&parent));
    assert!(parent.is_within(&parent));
    assert!(!parent.is_within(&child));
    assert!(!child.is_within(&unrelated));
}

#[test]
fn is_within_never_crosses_native_and_remote_locations() {
    let native = Location::local("/mnt/share/folder");
    let remote = Location::uri("sftp://user@host/mnt/share");

    assert!(!native.is_within(&remote));
    assert!(!remote.is_within(&native));
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

#[test]
fn smart_folder_locations_round_trip_ids() {
    let location = Location::smart_folder("recent-pdfs");
    assert!(location.is_smart_folder());
    assert_eq!(location.smart_folder_id(), Some("recent-pdfs"));
    assert_eq!(location.uri_value(), Some("strata-search://recent-pdfs"));
    assert_eq!(location.display_name(), "recent-pdfs");
    assert_eq!(location.parent(), None);
    assert_eq!(location.breadcrumbs(), vec![location.clone()]);
}

#[test]
fn empty_smart_folder_id_is_not_a_smart_folder() {
    let location = Location::smart_folder("");
    assert!(!location.is_smart_folder());
    assert_eq!(location.smart_folder_id(), None);
}

#[test]
fn non_smart_folder_uris_are_not_smart_folders() {
    assert!(!Location::uri("trash:///").is_smart_folder());
    assert!(!Location::uri("smb://server/share").is_smart_folder());
    assert!(!Location::local("/tmp").is_smart_folder());
}

#[test]
fn file_category_matches_extensions_and_directories() {
    use super::FileCategory;
    use std::path::Path;

    assert!(FileCategory::Folder.matches(Path::new("/home/user/docs"), true));
    assert!(!FileCategory::Folder.matches(Path::new("/home/user/doc.pdf"), false));

    assert!(FileCategory::Document.matches(Path::new("report.pdf"), false));
    assert!(FileCategory::Document.matches(Path::new("notes.md"), false));
    assert!(FileCategory::Document.matches(Path::new("sheet.xlsx"), false));
    assert!(!FileCategory::Document.matches(Path::new("photo.jpg"), false));

    assert!(FileCategory::Image.matches(Path::new("photo.png"), false));
    assert!(FileCategory::Image.matches(Path::new("art.SVG"), false));
    assert!(!FileCategory::Image.matches(Path::new("song.mp3"), false));

    assert!(FileCategory::Audio.matches(Path::new("track.flac"), false));
    assert!(FileCategory::Video.matches(Path::new("movie.mkv"), false));
    assert!(FileCategory::Archive.matches(Path::new("backup.tar.gz"), false));
    assert!(FileCategory::Code.matches(Path::new("main.rs"), false));
    assert!(FileCategory::Code.matches(Path::new("script.py"), false));
    assert!(FileCategory::Any.matches(Path::new("anything.bin"), false));
}

#[test]
fn date_and_size_constraints_match_accurately() {
    use super::{DateConstraint, SizeConstraint};

    let now = 10_000_000u64;
    let day_secs = 86400;

    let recent = now - day_secs * 2;
    let old = now - day_secs * 60;

    assert!(DateConstraint::WithinPastDays(7).matches(recent, now));
    assert!(!DateConstraint::WithinPastDays(7).matches(old, now));
    assert!(DateConstraint::OlderThanDays(30).matches(old, now));
    assert!(!DateConstraint::OlderThanDays(30).matches(recent, now));

    assert!(SizeConstraint::GreaterThan(1_000_000).matches(5_000_000, false));
    assert!(!SizeConstraint::GreaterThan(1_000_000).matches(500_000, false));
    assert!(SizeConstraint::LessThan(10_000_000).matches(5_000_000, false));
    assert!(!SizeConstraint::LessThan(10_000_000).matches(50_000_000, false));
    assert!(!SizeConstraint::GreaterThan(100).matches(1_000_000, true)); // directories do not match file size
}

#[test]
fn smart_query_rule_evaluates_combination() {
    use super::{DateConstraint, FileCategory, SizeConstraint, SmartQueryRule};
    use std::path::Path;

    let now = 10_000_000u64;
    let path = Path::new("/home/user/Pictures/wallpaper.png");
    let name = "wallpaper.png";

    let rule_kind = SmartQueryRule::Kind(FileCategory::Image);
    let rule_name = SmartQueryRule::NameContains("paper".into());
    let rule_size = SmartQueryRule::FileSize(SizeConstraint::GreaterThan(1000));
    let rule_date = SmartQueryRule::DateModified(DateConstraint::WithinPastDays(7));

    assert!(rule_kind.matches(path, name, false, 5000, now - 3600, now));
    assert!(rule_name.matches(path, name, false, 5000, now - 3600, now));
    assert!(rule_size.matches(path, name, false, 5000, now - 3600, now));
    assert!(rule_date.matches(path, name, false, 5000, now - 3600, now));

    let failing_name = SmartQueryRule::NameContains("photo".into());
    assert!(!failing_name.matches(path, name, false, 5000, now - 3600, now));
}
