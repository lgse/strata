// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn terminal_shortcut_prefers_one_selected_directory() {
    let entry = |name: &str, kind| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let directory = entry("selected", crate::model::EntryKind::Directory);
    let file = entry("notes.txt", crate::model::EntryKind::File);

    assert_eq!(
        selected_terminal_location(std::slice::from_ref(&directory)),
        Some(directory.location.clone())
    );
    assert_eq!(selected_terminal_location(&[directory, file.clone()]), None);
    assert_eq!(selected_terminal_location(&[file]), None);
    assert_eq!(selected_terminal_location(&[]), None);
}

#[cfg(unix)]
#[test]
fn terminal_directory_argument_preserves_native_path_bytes() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = Path::new(OsStr::from_bytes(b"/tmp/non-utf8-\xff"));

    assert_eq!(
        terminal_directory_argument(path).as_encoded_bytes(),
        b"--dir=/tmp/non-utf8-\xff"
    );
}

#[test]
fn global_activity_uses_the_latest_active_label() {
    let mut activity = GlobalActivityState::default();
    let connecting = activity.begin("Connecting…");
    let copying = activity.begin("Copying…");
    assert_eq!(activity.current_label(), Some("Copying…"));

    activity.finish(copying);
    assert_eq!(activity.current_label(), Some("Connecting…"));
    activity.finish(connecting);
    assert_eq!(activity.current_label(), None);
}

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
fn dialog_copy_wraps_long_paths_without_spaces() {
    let wrapped = wrap_dialog_text(&"a".repeat(80), 32);
    assert_eq!(
        wrapped.lines().map(str::len).collect::<Vec<_>>(),
        [32, 32, 16]
    );
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
fn location_input_credentials_are_one_shot_and_never_saved() {
    let (location, credentials) = credentials_from_location_input("smb://alice:secret@host/share")
        .expect("credential URI should parse");
    let credentials = credentials.expect("credentials should be separated");

    assert_eq!(location, "smb://alice@host/share");
    assert_eq!(credentials.username, "alice");
    assert_eq!(credentials.password, "secret");
    assert_eq!(credentials.save, gio::PasswordSave::Never);
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
fn authentication_failure_without_a_backend_prompt_gets_login_fields() {
    let location = Location::uri("smb://host/share");
    let details = MountPromptDetails::fallback(&location);
    assert!(details.message.contains("smb://host/share"));
    assert!(details.flags.contains(gio::AskPasswordFlags::NEED_USERNAME));
    assert!(details.flags.contains(gio::AskPasswordFlags::NEED_DOMAIN));
    assert!(details.flags.contains(gio::AskPasswordFlags::NEED_PASSWORD));
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
        is_hidden: false,
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
fn folder_peek_uses_visible_mode_bounds() {
    assert_eq!(
        peek_origin_bounds(BrowserMode::Columns),
        PeekOriginBounds::Column
    );
    assert_eq!(
        peek_origin_bounds(BrowserMode::Grid),
        PeekOriginBounds::Anchor
    );
    assert_eq!(
        peek_origin_bounds(BrowserMode::Explorer),
        PeekOriginBounds::Anchor
    );
}

#[test]
fn folder_peek_prefers_space_to_the_right_of_its_source_column() {
    assert_eq!(
        peek_horizontal_placement(100.0, 300.0, 800.0),
        Some(PeekPlacement {
            x: 408.0,
            side: PeekSide::Right,
        })
    );
}

#[test]
fn folder_peek_uses_the_left_only_when_it_fits_outside_the_source_column() {
    assert_eq!(
        peek_horizontal_placement(300.0, 300.0, 700.0),
        Some(PeekPlacement {
            x: 36.0,
            side: PeekSide::Left,
        })
    );
    assert_eq!(peek_horizontal_placement(200.0, 300.0, 700.0), None);
}

#[test]
fn folder_peek_animation_moves_toward_its_placement_side() {
    assert_eq!(
        peek_transition(PeekSide::Left),
        gtk::RevealerTransitionType::SlideLeft
    );
    assert_eq!(
        peek_transition(PeekSide::Right),
        gtk::RevealerTransitionType::SlideRight
    );
}

#[test]
fn folder_peek_animation_is_anchored_to_the_source_side() {
    assert_eq!(
        peek_horizontal_layout(
            PeekPlacement {
                x: 408.0,
                side: PeekSide::Right,
            },
            800.0,
        ),
        (gtk::Align::Start, 408, 0)
    );
    assert_eq!(
        peek_horizontal_layout(
            PeekPlacement {
                x: 36.0,
                side: PeekSide::Left,
            },
            700.0,
        ),
        (gtk::Align::End, 0, 408)
    );
}

#[test]
fn folder_peek_accepts_an_exact_viewport_fit() {
    assert_eq!(
        peek_horizontal_placement(0.0, 300.0, 564.0),
        Some(PeekPlacement {
            x: 308.0,
            side: PeekSide::Right,
        })
    );
}

#[test]
fn small_operations_delay_progress_while_large_or_unbounded_operations_show_it_immediately() {
    assert!(!should_show_progress_immediately(1));
    assert!(!should_show_progress_immediately(
        IMMEDIATE_PROGRESS_ITEM_COUNT - 1
    ));
    assert!(should_show_progress_immediately(
        IMMEDIATE_PROGRESS_ITEM_COUNT
    ));
    assert!(should_show_progress_immediately(0));
}

#[test]
fn delete_confirmation_direction_keys_choose_an_action() {
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::Left),
        Some(DeleteConfirmationFocus::Cancel)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::h),
        Some(DeleteConfirmationFocus::Cancel)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::Right),
        Some(DeleteConfirmationFocus::Confirm)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::l),
        Some(DeleteConfirmationFocus::Confirm)
    );
    assert_eq!(delete_confirmation_focus_target(gtk::gdk::Key::Tab), None);
}

#[test]
fn vim_focus_keys_map_to_dialog_directions() {
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
        is_hidden: false,
    };

    assert!(entry_supports_quick_preview(&entry(
        "photo.png",
        crate::model::EntryKind::File,
    )));
    assert!(entry_supports_quick_preview(&entry(
        "notes.txt",
        crate::model::EntryKind::FileSymbolicLink,
    )));
    assert!(entry_supports_quick_preview(&entry(
        ".steampath",
        crate::model::EntryKind::File,
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
    assert!(entry_responds_to_preview_click(&supported, true));
    assert!(!entry_responds_to_preview_click(&supported, false));
    assert!(!entry_responds_to_preview_click(&unsupported, true));
    assert!(!entry_responds_to_preview_click(&directory, true));
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
fn incoming_file_lists_sanitize_remote_credentials() {
    let files = gtk::gdk::FileList::from_array(&[
        gio::File::for_uri("smb://user%3Asecret@host/share"),
        gio::File::for_uri("smb://user%3Bpassword=secret@host/share"),
        gio::File::for_uri("sftp://user@host/home/user/video.mp4"),
    ]);

    let locations = locations_from_file_list_value(&files.to_value())
        .expect("file list should contain sanitized locations");
    let uris = locations
        .iter()
        .map(|location| {
            location
                .uri_value()
                .expect("remote location should have a URI")
                .trim_end_matches('/')
        })
        .collect::<Vec<_>>();
    assert_eq!(
        uris,
        [
            "smb://user@host/share",
            "smb://user@host/share",
            "sftp://user@host/home/user/video.mp4",
        ]
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
        is_hidden: false,
    };

    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three")]),
        "one, two, three"
    );
    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three"), entry("four")]),
        "one, two, three, …"
    );
    let summary = selected_items_summary(&[
        entry("a-very-long-file-name-that-would-expand-the-context-menu"),
        entry("another-very-long-file-name-that-would-expand-the-menu"),
    ]);
    assert_eq!(
        summary.chars().count(),
        ITEM_CONTEXT_SUMMARY_MAX_CHARS as usize
    );
    assert!(summary.ends_with('…'));
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
fn archive_names_strip_only_the_selected_dotted_extension() {
    assert_eq!(
        normalized_archive_name("backup.zip", ArchiveFormat::Zip),
        "backup"
    );
    assert_eq!(
        normalized_archive_name("backupzip", ArchiveFormat::Zip),
        "backupzip"
    );
    assert_eq!(
        normalized_archive_name("backup.tar.gz", ArchiveFormat::TarGz),
        "backup"
    );
    assert!(
        validate_basename(&normalized_archive_name(
            "../outside.zip",
            ArchiveFormat::Zip
        ))
        .is_err()
    );
}

#[test]
fn archive_collisions_use_the_final_name() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let destination = Location::local(root.path());
    assert!(!archive_has_collision(&destination, "archive.zip"));
    std::fs::write(root.path().join("archive.zip"), b"existing")?;
    assert!(archive_has_collision(&destination, "archive.zip"));
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
fn path_suggestions_list_only_matching_folders() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("strata-path-suggestions-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Documents"))?;
    std::fs::create_dir_all(root.join("Downloads"))?;
    std::fs::create_dir_all(root.join("relative/Documents"))?;
    std::fs::write(root.join("Document.txt"), b"not a folder")?;
    let home = root.join("home");

    let suggestions = path_suggestions(&format!("{}/Doc", root.display()), &root, &home);
    assert_eq!(suggestions, vec![root.join("Documents")]);

    let relative = path_suggestions("relative/Doc", &root, &home);
    assert_eq!(relative, vec![root.join("relative/Documents")]);
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
fn pointer_preview_handler_ignores_double_click_activation() {
    assert!(should_preview_pointer_press(1, false, false, false));
    assert!(!should_preview_pointer_press(2, false, false, false));
    assert!(!should_preview_pointer_press(1, true, false, false));
    assert!(!should_preview_pointer_press(1, false, true, false));
    assert!(!should_preview_pointer_press(1, false, false, true));
}

#[test]
fn pointer_activation_respects_entry_type_click_count_and_modifiers() {
    let activation = ClickActivation {
        files: ClickCount::Two,
        folders: ClickCount::One,
    };

    assert!(should_activate_single_click(
        1, true, activation, false, false, false
    ));
    assert!(!should_activate_single_click(
        1, false, activation, false, false, false
    ));
    assert!(!should_activate_single_click(
        2, true, activation, false, false, false
    ));
    assert!(!should_activate_single_click(
        1, true, activation, true, false, false
    ));
    assert!(!should_activate_single_click(
        1, true, activation, false, true, false
    ));
    assert!(!should_activate_single_click(
        1, true, activation, false, false, true
    ));
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
    assert_eq!(
        paste_destination_depth(BrowserMode::Columns, Some(1), Some(2), 3),
        Some(1)
    );
    assert_eq!(
        paste_destination_depth(BrowserMode::Columns, None, Some(1), 3),
        Some(2)
    );
    assert_eq!(
        paste_destination_depth(BrowserMode::Columns, Some(4), Some(1), 3),
        Some(2)
    );
    assert_eq!(
        paste_destination_depth(BrowserMode::Columns, None, None, 0),
        None
    );
}

#[test]
fn single_pane_paste_uses_the_active_browser_depth() {
    for mode in [BrowserMode::Grid, BrowserMode::Explorer] {
        assert_eq!(paste_destination_depth(mode, None, Some(2), 0), Some(2));
    }
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
    assert!(!same_locations(
        &[
            Location::local("/fixture/first"),
            Location::local("/fixture/first")
        ],
        &[
            Location::local("/fixture/first"),
            Location::local("/fixture/second")
        ]
    ));
}

#[test]
fn completed_moves_are_removed_from_the_cut_list() {
    let first = Location::local("/fixture/first");
    let second = Location::local("/fixture/second");
    let third = Location::local("/fixture/third");
    let mut cut = vec![first.clone(), second.clone(), third.clone()];

    retain_untransferred(&mut cut, &[first, third]);

    assert_eq!(cut, vec![second]);
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

fn unique_fixture_root(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-trash-{label}-{unique}"))
}

#[test]
fn trash_summary_reports_truncated_once_the_entry_budget_is_exceeded() {
    let root = unique_fixture_root("entry-budget");
    std::fs::create_dir_all(root.join("sub")).expect("the trash fixture should be created");
    for index in 0..5 {
        std::fs::write(
            root.join("sub").join(format!("file-{index}.txt")),
            b"content",
        )
        .expect("the trash fixture file should be written");
    }

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        1,
        MAX_TRASH_DEPTH,
        TRASH_TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated,
        "exceeding the entry budget should be reported"
    );
    assert_eq!(
        summary.item_count, 1,
        "measurement should stop counting once the entry budget is reached"
    );
}

#[test]
fn trash_summary_reports_truncated_once_the_time_budget_is_exceeded() {
    let root = unique_fixture_root("time-budget");
    std::fs::create_dir_all(root.join("sub")).expect("the trash fixture should be created");
    std::fs::write(root.join("sub").join("file.txt"), b"content")
        .expect("the trash fixture file should be written");

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        usize::MAX,
        MAX_TRASH_DEPTH,
        Duration::from_nanos(1),
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated,
        "an exhausted time budget should stop measurement and report truncation"
    );
}

#[test]
fn trash_summary_does_not_descend_past_the_depth_budget() {
    let root = unique_fixture_root("depth-budget");
    std::fs::create_dir_all(root.join("sub/nested")).expect("the trash fixture should be created");
    std::fs::write(root.join("sub/nested/deep.txt"), b"content")
        .expect("the trash fixture file should be written");

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        usize::MAX,
        1,
        TRASH_TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert!(
        summary.truncated,
        "descending past the depth budget should be reported"
    );
    assert_eq!(
        summary.item_count, 2,
        "entries past the depth budget should not be counted (root/sub and sub/nested only)"
    );
}

#[test]
fn trash_summary_treats_an_inaccessible_subdirectory_as_truncated_not_fatal() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_fixture_root("inaccessible");
    std::fs::create_dir_all(root.join("blocked")).expect("the trash fixture should be created");
    std::fs::create_dir_all(root.join("visible")).expect("the trash fixture should be created");
    std::fs::write(root.join("visible/needle.txt"), b"content")
        .expect("the trash fixture file should be written");
    std::fs::set_permissions(root.join("blocked"), std::fs::Permissions::from_mode(0o000))
        .expect("the fixture directory's permissions should be restrictable");
    let running_as_root = std::fs::read_dir(root.join("blocked")).is_ok();

    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        MAX_TRASH_ENTRIES,
        MAX_TRASH_DEPTH,
        TRASH_TIME_BUDGET,
    ));
    let _ = std::fs::set_permissions(root.join("blocked"), std::fs::Permissions::from_mode(0o755));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect(
        "an inaccessible subdirectory should degrade gracefully, not fail the whole measurement",
    );
    if !running_as_root {
        assert!(
            summary.truncated,
            "the inaccessible branch should be reported as truncated"
        );
        assert_eq!(
            summary.item_count, 3,
            "blocked (uncounted contents) + visible + needle.txt"
        );
    }
}

#[test]
fn trash_summary_treats_a_directory_removed_before_measurement_as_truncated_not_fatal() {
    let root = unique_fixture_root("changing-tree");
    let vanishing = root.join("vanishing");
    std::fs::create_dir_all(&vanishing).expect("the trash fixture should be created");
    std::fs::write(vanishing.join("inner.txt"), b"content")
        .expect("the trash fixture file should be written");

    // Capture the directory's FileInfo the way the parent-level enumeration in
    // `summarize_trash_with_budget` would, then remove it from under that handle. This models a
    // directory that changes or disappears between being observed and being measured.
    let file = gio::File::for_path(&vanishing);
    let context = glib::MainContext::new();
    let info = context
        .block_on(file.query_info_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        ))
        .expect("querying the fixture directory's info should succeed while it still exists");
    std::fs::remove_dir_all(&vanishing).expect("the fixture directory should be removable");

    let result = context.block_on(measure_trash_entry(
        file,
        info,
        0,
        Rc::new(Cell::new(0)),
        Instant::now() + TRASH_TIME_BUDGET,
        MAX_TRASH_ENTRIES,
        MAX_TRASH_DEPTH,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture root should be removed");

    let (_, _, truncated) = result.expect(
        "a directory removed after being observed should degrade gracefully, not fail the whole measurement",
    );
    assert!(
        truncated,
        "measuring an entry that vanished before recursion should be reported as truncated"
    );
}

#[test]
fn aborting_a_trash_measurement_stops_it_mid_flight() {
    let root = unique_fixture_root("abort-mid-flight");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // `next_files_future` batches 64 entries at a time, so 200 files force several suspension
    // points, giving a real window to observe partial progress before the walk would finish.
    let total_files = 200;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    // `spawn_local` and manual `iteration()` polling (unlike `block_on`) require this thread to
    // own the context, hence the explicit acquire via `with_thread_default` below.
    let context = glib::MainContext::new();
    let (progress_before_abort, progress_after_abort) = context
        .with_thread_default(|| {
            let info = context
                .block_on(gio::File::for_path(&root).query_info_future(
                    TRASH_ATTRIBUTES,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                ))
                .expect("querying the fixture directory's info should succeed");

            let visited = Rc::new(Cell::new(0_usize));
            let task = context.spawn_local(measure_trash_entry(
                gio::File::for_path(&root),
                info,
                0,
                visited.clone(),
                Instant::now() + TRASH_TIME_BUDGET,
                MAX_TRASH_ENTRIES,
                MAX_TRASH_DEPTH,
            ));

            // Drive the loop only until the walk has made some real progress (at least one batch
            // beyond the root directory itself), then abort immediately -- this is genuinely
            // mid-flight since one batch (64) is far short of the full tree (1 + 200), regardless
            // of exactly how many main-loop iterations it took to get there.
            for _ in 0..1_000 {
                if visited.get() > 1 {
                    break;
                }
                context.iteration(true);
            }
            let progress_before_abort = visited.get();

            task.abort();
            for _ in 0..20 {
                context.iteration(false);
            }

            (progress_before_abort, visited.get())
        })
        .expect("a freshly created main context should be acquirable as thread-default");
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    assert!(
        progress_before_abort > 1 && progress_before_abort < 1 + total_files,
        "the walk should have made partial, not complete, progress before it is aborted"
    );
    assert_eq!(
        progress_after_abort, progress_before_abort,
        "aborting mid-flight should stop the walk from making any further progress"
    );
}

#[test]
fn trash_summary_stops_enumerating_the_root_once_the_measurement_budget_is_reached() {
    let root = unique_fixture_root("root-budget-stop");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch (64 entries), so a walk that kept fetching
    // further batches after the budget was spent would still show up as a much larger count.
    let total_files = 150;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let max_entries = 5;
    let summary = glib::MainContext::new()
        .block_on(summarize_trash_with_budget(
            &gio::File::for_path(&root),
            max_entries,
            MAX_TRASH_DEPTH,
            TRASH_TIME_BUDGET,
        ))
        .expect("a plain directory tree should measure without error");
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    assert!(
        summary.truncated,
        "exceeding the measurement budget should still be reported"
    );
    assert_eq!(
        summary.item_count, max_entries,
        "root enumeration should stop as soon as the budget is spent, not keep requesting \
         further `next_files_future` batches for the remaining {total_files} entries"
    );
}

#[test]
fn empty_trash_deletes_every_top_level_entry_in_bounded_batches() {
    let root = unique_fixture_root("empty-trash-streaming");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch, so this also exercises the batch-to-batch loop.
    let total_files = 150;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let last_progress = Rc::new(Cell::new(0_usize));
    let progress_tick = last_progress.clone();
    let outcome = glib::MainContext::new()
        .block_on(empty_trash(&gio::File::for_path(&root), move |processed| {
            progress_tick.set(processed);
        }))
        .expect("a plain directory tree should empty without error");
    std::fs::remove_dir_all(&root).expect("the trash fixture root should be removed");

    assert_eq!(
        outcome.deleted, total_files,
        "every top-level entry must be deleted, independent of any prior measurement budget"
    );
    assert_eq!(outcome.failed, 0);
    assert_eq!(
        last_progress.get(),
        total_files,
        "progress should account for every entry once the walk finishes"
    );
}

#[test]
fn aborting_empty_trash_stops_deletion_mid_flight() {
    let root = unique_fixture_root("abort-empty-trash-mid-flight");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch (64 entries), so aborting after the first batch's
    // progress callback is genuinely mid-flight, not the whole walk finishing in one step.
    let total_files = 200;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let context = glib::MainContext::new();
    let (progress_before_abort, progress_after_abort) = context
        .with_thread_default(|| {
            let progress = Rc::new(Cell::new(0_usize));
            let progress_tick = progress.clone();
            let trash_root = gio::File::for_path(&root);
            let task = context.spawn_local(async move {
                empty_trash(&trash_root, move |processed| progress_tick.set(processed)).await
            });

            // Drive the loop only until the first batch has reported progress, then abort
            // immediately -- with 200 files and a 64-entry batch size, that's genuinely
            // mid-flight regardless of exactly how many main-loop iterations it took to get there.
            for _ in 0..1_000 {
                if progress.get() > 0 {
                    break;
                }
                context.iteration(true);
            }
            let progress_before_abort = progress.get();

            task.abort();
            for _ in 0..20 {
                context.iteration(false);
            }

            (progress_before_abort, progress.get())
        })
        .expect("a freshly created main context should be acquirable as thread-default");
    let remaining = std::fs::read_dir(&root)
        .expect("the fixture root should still exist")
        .count();
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    assert!(
        progress_before_abort > 0 && progress_before_abort < total_files,
        "the deletion should have made partial, not complete, progress before it is aborted"
    );
    assert_eq!(
        progress_after_abort, progress_before_abort,
        "aborting mid-flight should stop the deletion from making any further progress"
    );
    assert!(
        remaining > 0,
        "aborting mid-flight should leave undeleted entries behind, not finish the walk anyway"
    );
}

#[test]
fn trash_summary_does_not_stop_enumerating_siblings_after_one_branch_is_depth_truncated() {
    let root = unique_fixture_root("sibling-depth-truncation");
    let sibling_count = 80;
    // Nested one level under "parent" so these are children of a directory that
    // `enumerate_trash_directory` recurses into, not top-level entries of `root` itself (which
    // are always fully enumerated regardless of budget after the earlier deletion-worklist fix).
    for index in 0..sibling_count {
        std::fs::create_dir_all(
            root.join("parent")
                .join(format!("sub-{index:03}"))
                .join("inner"),
        )
        .expect("the trash fixture should be created");
    }

    // With max_depth 1, every "sub-N" directory individually hits the depth cap when deciding
    // whether to recurse into its own "inner" child -- that is a branch-local condition, unrelated
    // to its siblings. It used to be conflated with the shared budget being spent, which stopped
    // scanning further `next_files_future` batches entirely: with 80 siblings and a 64-entry batch
    // size, that undercounted "parent" to 1 (itself) + 64 (first batch only) = 65.
    let summary = glib::MainContext::new().block_on(summarize_trash_with_budget(
        &gio::File::for_path(&root),
        usize::MAX,
        1,
        TRASH_TIME_BUDGET,
    ));
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    let summary = summary.expect("a plain directory tree should measure without error");
    assert_eq!(
        summary.item_count,
        1 + sibling_count,
        "every sibling should be counted (1 for \"parent\" plus one per sub-N directory), \
         not just the first next_files_future batch"
    );
}

#[test]
fn entry_model_value_encodes_hidden_state_and_preserves_display_name() {
    let visible = FileEntry {
        location: Location::local("/fixture/photo"),
        native_name: "photo".into(),
        display_name: "photo".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let hidden = FileEntry {
        location: Location::local("/fixture/.config"),
        native_name: ".config".into(),
        display_name: ".config".into(),
        kind: crate::model::EntryKind::Directory,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: true,
    };

    let encoded_visible = entry_model_value(&visible);
    let encoded_hidden = entry_model_value(&hidden);

    assert_eq!(encoded_visible, "fv\tphoto");
    assert_eq!(encoded_hidden, "dh\t.config");

    assert!(!model_is_hidden(&encoded_visible));
    assert!(model_is_hidden(&encoded_hidden));

    assert_eq!(model_display_name(&encoded_visible), "photo");
    assert_eq!(model_display_name(&encoded_hidden), ".config");
}

#[test]
fn shell_escape_path_escapes_spaces_and_metacharacters_but_not_filename_unicode() {
    assert_eq!(
        shell_escape_path(Path::new("/mnt/Mass 1/Movies\u{2044}TV")),
        "/mnt/Mass\\ 1/Movies\u{2044}TV"
    );
    assert_eq!(
        shell_escape_path(Path::new("/tmp/archive (final) v1.2.tar.gz")),
        "/tmp/archive\\ \\(final\\)\\ v1.2.tar.gz"
    );
    assert_eq!(
        shell_escape_path(Path::new("/home/user/plain")),
        "/home/user/plain"
    );
}

#[test]
fn copy_path_text_adds_trailing_slash_to_directories_only() {
    let directory = Location::local("/mnt/Mass 1/Movies\u{2044}TV");
    assert_eq!(
        copy_path_text(&directory, true),
        "/mnt/Mass\\ 1/Movies\u{2044}TV/"
    );
    assert_eq!(
        copy_path_text(&directory, false),
        "/mnt/Mass\\ 1/Movies\u{2044}TV"
    );
}

#[test]
fn copy_path_text_keeps_uris_unescaped() {
    let remote = Location::uri("smb://server/share/folder");
    assert_eq!(copy_path_text(&remote, true), "smb://server/share/folder");
}

#[test]
fn shell_escape_path_preserves_newlines_and_single_quotes() {
    assert_eq!(
        shell_escape_path(Path::new("/tmp/line\nbob's notes")),
        "'/tmp/line\nbob'\\''s notes'"
    );
}

#[test]
fn pinning_requires_an_available_non_trash_directory() {
    let entry = |location, kind| FileEntry {
        location,
        native_name: "item".into(),
        display_name: "item".into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let directory = entry(
        Location::local("/fixture/folder"),
        crate::model::EntryKind::Directory,
    );
    let file = entry(
        Location::local("/fixture/file"),
        crate::model::EntryKind::File,
    );
    let trash_directory = entry(
        Location::uri("trash:///deleted-folder"),
        crate::model::EntryKind::Directory,
    );

    assert!(can_pin_entry(&directory, PinStatus::Available));
    assert!(!can_pin_entry(&directory, PinStatus::Pinned));
    assert!(!can_pin_entry(&directory, PinStatus::Unavailable));
    assert!(!can_pin_entry(&file, PinStatus::Available));
    assert!(!can_pin_entry(&trash_directory, PinStatus::Available));
}
