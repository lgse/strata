// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn file_sizes_use_compact_decimal_units() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(1_000_000), "1 MB");
    assert_eq!(format_file_size(2_500_000_000), "2.5 GB");
}

#[test]
fn an_empty_name_is_not_flagged_as_an_error() {
    assert!(basename_field_error("bad/name").is_some());
    assert!(
        basename_field_error("").is_none(),
        "an empty field is the normal starting state, not a user mistake"
    );
}

#[test]
fn dialog_copy_wraps_at_word_boundaries() {
    let wrapped = wrap_dialog_text(
        "Those credentials were not accepted. Check the username and password.",
        32,
    );
    assert_eq!(
        wrapped,
        "Those credentials were not\naccepted. Check the username and\npassword."
    );
    assert!(wrapped.lines().all(|line| line.chars().count() <= 32));
}

#[test]
fn password_storage_selection_maps_to_gio_values() {
    assert_eq!(password_save_for_selection(0), gio::PasswordSave::Never);
    assert_eq!(
        password_save_for_selection(1),
        gio::PasswordSave::ForSession
    );
    assert_eq!(
        password_save_for_selection(2),
        gio::PasswordSave::Permanently
    );
    assert_eq!(password_save_for_selection(99), gio::PasswordSave::Never);
}

#[test]
fn remote_permission_denials_are_treated_as_authentication_failures() {
    let denied = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "Permission denied");
    let smb_denied = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Failed to mount Windows share: Permission denied",
    );
    let remote = Location::uri("smb://host/share");
    assert!(mount_error_is_authentication_failure(&remote, &denied));
    assert!(mount_error_is_authentication_failure(&remote, &smb_denied,));
    assert!(!mount_error_is_authentication_failure(
        &Location::local("/root"),
        &denied,
    ));
}

#[test]
fn cancelling_the_credential_prompt_produces_no_error_message() {
    let location = Location::uri("smb://host/share");
    for kind in [gio::IOErrorEnum::Cancelled, gio::IOErrorEnum::FailedHandled] {
        let error = glib::Error::new(kind, "cancelled by the user");
        assert_eq!(mount_failure_message(&location, &error), None);
    }
}

#[test]
fn a_missing_backend_reports_which_package_to_install() {
    let location = Location::uri("smb://host/share");
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "no handler for smb");
    let message = mount_failure_message(&location, &error).expect("should report a message");
    assert!(message.contains("gvfs-smb"));
}

#[test]
fn a_genuine_mount_failure_still_reports_an_error() {
    let location = Location::uri("smb://host/share");
    let error = glib::Error::new(gio::IOErrorEnum::HostNotFound, "no route to host");
    let message = mount_failure_message(&location, &error).expect("should report a message");
    assert!(message.contains("no route to host"));
}

#[test]
fn inline_rename_selects_the_stem_but_keeps_the_extension() {
    assert_eq!(rename_stem_end("report.txt"), 6);
    assert_eq!(rename_stem_end("archive.tar.gz"), 11);
    assert_eq!(rename_stem_end("README"), 6);
    assert_eq!(rename_stem_end(".gitignore"), 10);
}

#[test]
fn delete_confirmation_labels_distinguish_files_and_folders() {
    let file = FileEntry {
        location: Location::local("/fixture/file.txt"),
        native_name: "file.txt".into(),
        display_name: "file.txt".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(10),
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    };
    let mut folder = file.clone();
    folder.kind = crate::model::EntryKind::Directory;

    assert_eq!(item_count_label(1), "1 item");
    assert_eq!(item_count_label(2), "2 items");
    assert_eq!(entry_kind_summary(std::slice::from_ref(&file)), "1 file");
    assert_eq!(entry_kind_summary(&[file.clone(), file.clone()]), "2 files");
    assert_eq!(entry_kind_summary(&[folder.clone()]), "1 folder");
    assert_eq!(entry_kind_summary(&[file, folder]), "2 items");
}

#[test]
fn only_the_trash_root_uses_the_aggregate_properties_size() {
    assert!(is_trash_root(&Location::uri("trash:///")));
    assert!(!is_trash_root(&Location::uri("trash:///photo.png")));
    assert!(!is_trash_root(&Location::local(
        "/home/user/.local/share/Trash"
    )));
}

#[test]
fn quick_preview_is_offered_only_for_supported_files() {
    let entry = |name: &str, kind| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    };

    assert!(entry_supports_quick_preview(&entry(
        "photo.png",
        crate::model::EntryKind::File,
    )));
    assert!(entry_supports_quick_preview(&entry(
        "notes.txt",
        crate::model::EntryKind::FileSymbolicLink,
    )));
    assert!(!entry_supports_quick_preview(&entry(
        "archive.zip",
        crate::model::EntryKind::File,
    )));
    assert!(!entry_supports_quick_preview(&entry(
        "photos.png",
        crate::model::EntryKind::Directory,
    )));

    let supported = entry("photo.png", crate::model::EntryKind::File);
    let unsupported = entry("archive.zip", crate::model::EntryKind::File);
    let directory = entry("photos", crate::model::EntryKind::Directory);
    assert!(entry_responds_to_single_click(&supported, true));
    assert!(!entry_responds_to_single_click(&supported, false));
    assert!(!entry_responds_to_single_click(&unsupported, true));
    assert!(entry_responds_to_single_click(&directory, false));
}

#[test]
fn incoming_file_lists_preserve_local_and_remote_locations() {
    let files = gtk::gdk::FileList::from_array(&[
        gio::File::for_path("/fixture/photo.raw"),
        gio::File::for_uri("sftp://host.example/home/user/video.mp4"),
    ]);
    let value = files.to_value();

    assert_eq!(
        locations_from_file_list_value(&value),
        Some(vec![
            Location::local("/fixture/photo.raw"),
            Location::uri("sftp://host.example/home/user/video.mp4"),
        ])
    );
}

#[test]
fn local_file_drops_prefer_move_while_external_drops_prefer_copy() {
    let both = gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE;

    assert_eq!(
        preferred_file_drop_action(both, true),
        gtk::gdk::DragAction::MOVE
    );
    assert_eq!(
        preferred_file_drop_action(both, false),
        gtk::gdk::DragAction::COPY
    );
    assert_eq!(
        preferred_file_drop_action(gtk::gdk::DragAction::MOVE, false),
        gtk::gdk::DragAction::MOVE
    );
}

#[test]
fn multi_selection_summary_lists_at_most_three_names() {
    let entry = |name: &str| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    };

    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three")]),
        "one, two, three"
    );
    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three"), entry("four")]),
        "one, two, three, …"
    );
}

#[test]
fn trash_locations_include_the_root_and_descendants() {
    assert!(is_trash_location(&Location::uri("trash:///")));
    assert!(is_trash_location(&Location::uri("trash:///folder")));
    assert!(!is_trash_location(&Location::local(
        "/home/example/.local/share/Trash"
    )));
}

#[test]
fn transfer_collisions_detect_existing_destination_items() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!("strata-collision-test-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let destination = root.join("destination");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::create_dir_all(&destination)?;
    let source = source_dir.join("photo.jpg");
    std::fs::write(&source, b"new")?;

    assert!(!transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));
    std::fs::write(destination.join("photo.jpg"), b"old")?;
    assert!(transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn destination_paths_expand_home_and_relative_input() {
    let base = std::path::Path::new("/work/current");
    let home = std::path::Path::new("/home/example");

    assert_eq!(resolve_destination_path("~", base, home), home);
    assert_eq!(
        resolve_destination_path("~/Documents", base, home),
        home.join("Documents")
    );
    assert_eq!(
        resolve_destination_path("../Archive", base, home),
        base.join("../Archive")
    );
    assert_eq!(
        resolve_destination_path("/tmp/export", base, home),
        std::path::Path::new("/tmp/export")
    );
}

#[test]
fn directory_autocomplete_suggests_only_matching_folders() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!("strata-path-suggestions-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Documents"))?;
    std::fs::create_dir_all(root.join("Downloads"))?;
    std::fs::write(root.join("Document.txt"), b"not a folder")?;

    let suggestions = directory_suggestions(&format!("{}/Doc", root.display()), &root, &root);

    assert_eq!(suggestions, vec![root.join("Documents")]);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn properties_permissions_are_formatted_symbolically_and_numerically() {
    assert_eq!(format_permissions(0o100774), "-rwxrwxr--  774");
    assert_eq!(format_permissions(0o040755), "drwxr-xr-x  755");
}

#[test]
fn properties_paths_abbreviate_the_home_directory() {
    let home = glib::home_dir();

    assert_eq!(compact_display_path(&Location::local(&home)), "~");
    assert_eq!(
        compact_display_path(&Location::local(home.join("Documents/report.txt"))),
        "~/Documents/report.txt"
    );
    assert_eq!(
        compact_display_path(&Location::uri("trash:///example")),
        "trash:///example"
    );
}

#[test]
fn file_names_map_to_specific_lucide_icons() {
    assert_eq!(icon_for_name("setup.sh"), crate::assets::icons::TERMINAL);
    assert_eq!(icon_for_name("photo.webp"), crate::assets::icons::PICTURES);
    assert_eq!(icon_for_name("movie.mkv"), crate::assets::icons::VIDEOS);
    assert_eq!(icon_for_name("source.rs"), crate::assets::icons::FILE_CODE);
    assert_eq!(
        icon_for_name("backup.tar"),
        crate::assets::icons::FILE_ARCHIVE
    );
    assert_eq!(icon_for_name("README.md"), crate::assets::icons::DOCUMENTS);
}

#[test]
fn pressing_an_item_in_a_multi_selection_preserves_the_drag_group() {
    assert!(should_preserve_drag_selection(true, 2));
    assert!(should_preserve_drag_selection(true, 8));
    assert!(!should_preserve_drag_selection(true, 1));
    assert!(!should_preserve_drag_selection(false, 4));
}

#[test]
fn paste_prefers_the_hovered_pane_then_the_deepest_pane() {
    assert_eq!(paste_destination_depth(Some(1), 3), Some(1));
    assert_eq!(paste_destination_depth(None, 3), Some(2));
    assert_eq!(paste_destination_depth(Some(4), 3), Some(2));
    assert_eq!(paste_destination_depth(None, 0), None);
}

#[test]
fn new_folder_prefers_the_hovered_pane_then_falls_back_safely() {
    assert_eq!(
        new_folder_destination_depth(Some(1), Some(0), Some(2), 3),
        Some(1)
    );
    assert_eq!(
        new_folder_destination_depth(None, Some(0), Some(2), 3),
        Some(0)
    );
    assert_eq!(
        new_folder_destination_depth(Some(4), Some(5), Some(1), 3),
        Some(1)
    );
    assert_eq!(new_folder_destination_depth(None, None, None, 3), Some(2));
    assert_eq!(new_folder_destination_depth(None, None, None, 0), None);
}

#[test]
fn cut_clipboard_locations_match_regardless_of_order() {
    let first = Location::local("/fixture/first");
    let second = Location::local("/fixture/second");

    assert!(same_locations(
        &[first.clone(), second.clone()],
        &[second, first]
    ));
    assert!(!same_locations(&[], &[]));
    assert!(!same_locations(
        &[Location::local("/fixture/first")],
        &[Location::local("/fixture/other")]
    ));
}

#[test]
fn single_pane_modes_reserve_half_for_preview_sizing() {
    assert_eq!(single_pane_preview_reservation(800), 400);
    assert_eq!(single_pane_preview_reservation(0), 0);
}

#[test]
fn pane_resizing_preserves_the_initial_minimum_width() {
    assert_eq!(resized_column_width(COLUMN_WIDTH, -80.0), COLUMN_WIDTH);
    assert_eq!(resized_column_width(COLUMN_WIDTH, 75.0), 375);
    assert_eq!(resized_column_width(420, -20.0), 400);
}

#[test]
fn reveal_target_scrolls_only_enough_to_show_the_new_column() {
    assert_eq!(
        horizontal_reveal_target(0.0, 900.0, 0.0, 1_200.0, 900.0, 1_200.0),
        300.0
    );
}

#[test]
fn reveal_target_is_stable_when_the_column_is_already_visible() {
    assert_eq!(
        horizontal_reveal_target(300.0, 900.0, 0.0, 1_500.0, 900.0, 1_200.0),
        300.0
    );
}

#[test]
fn reveal_target_can_scroll_back_to_an_earlier_column() {
    assert_eq!(
        horizontal_reveal_target(600.0, 900.0, 0.0, 1_500.0, 300.0, 600.0),
        300.0
    );
}
