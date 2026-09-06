// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn entry_matching_uses_the_display_name_and_hidden_flag() {
    for (value, show_hidden, query, expected) in [
        ("fv\tAlpha.txt", false, "alpha", true),
        ("fh\tAlpha.txt", false, "alpha", false),
        ("fh\tAlpha.txt", true, "alpha", true),
        ("fv\tAlpha.txt", false, "fv", false),
        ("dv\tFolder", false, "", true),
        ("dh\tFolder", false, "", false),
        ("fv\tÉcole\tNotes", false, "école\tnotes", true),
        ("plain name", false, "name", true),
        ("fv\tAlpha.txt", true, "beta", false),
    ] {
        assert_eq!(
            entry_matches(value, show_hidden, query),
            expected,
            "{value:?}, {show_hidden}, {query:?}"
        );
    }
}
use crate::model::{FileEntry, Location};
use gtk::gio;
use std::path::Path;

#[test]
fn file_sizes_use_compact_decimal_units() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(1_000_000), "1 MB");
    assert_eq!(format_file_size(2_500_000_000), "2.5 GB");
}

#[test]
fn delete_confirmation_labels_distinguish_files_and_folders() {
    let file = FileEntry {
        location: Location::local("/fixture/file.txt"),
        native_name: "file.txt".into(),
        thumbnail_path: None,
        display_name: "file.txt".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(10),
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
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
fn quick_preview_is_offered_only_for_supported_files() {
    let entry = |name: &str, kind| FileEntry {
        thumbnail_path: None,
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    };

    assert!(crate::ui::preview::entry_supports_quick_preview(&entry(
        "photo.png",
        crate::model::EntryKind::File,
    )));
    assert!(crate::ui::preview::entry_supports_quick_preview(&entry(
        "notes.txt",
        crate::model::EntryKind::FileSymbolicLink,
    )));
    assert!(crate::ui::preview::entry_supports_quick_preview(&entry(
        ".steampath",
        crate::model::EntryKind::File,
    )));
    assert!(!crate::ui::preview::entry_supports_quick_preview(&entry(
        "archive.zip",
        crate::model::EntryKind::File,
    )));
    assert!(!crate::ui::preview::entry_supports_quick_preview(&entry(
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
fn printing_is_offered_for_text_code_images_and_pdfs() {
    let entry = |name: &str, kind| FileEntry {
        thumbnail_path: None,
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        mode: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };

    for name in [
        "notes.txt",
        "main.rs",
        "settings.toml",
        "photo.png",
        "guide.pdf",
    ] {
        assert!(entry_supports_printing(&entry(
            name,
            crate::model::EntryKind::File
        )));
    }
    assert!(!entry_supports_printing(&entry(
        "archive.zip",
        crate::model::EntryKind::File,
    )));
    assert!(!entry_supports_printing(&entry(
        "notes.txt",
        crate::model::EntryKind::Directory,
    )));
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
fn entry_model_value_encodes_hidden_state_and_preserves_display_name() {
    let visible = FileEntry {
        location: Location::local("/fixture/photo"),
        native_name: "photo".into(),
        thumbnail_path: None,
        display_name: "photo".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
    };
    let hidden = FileEntry {
        location: Location::local("/fixture/.config"),
        native_name: ".config".into(),
        thumbnail_path: None,
        display_name: ".config".into(),
        kind: crate::model::EntryKind::Directory,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: true,
        mode: crate::model::MetadataValue::Unknown,
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
fn type_groups_name_folders_and_broken_links_directly() {
    assert_eq!(model_type_group("dv\tprojects"), FOLDER_TYPE_GROUP);
    assert_eq!(model_type_group("dv\tlinked"), FOLDER_TYPE_GROUP);
    assert_eq!(model_type_group("xv\tdangling"), "Broken link");
}

#[test]
fn files_of_an_unrecognized_type_share_one_group() {
    assert_eq!(model_type_group("fv\tblob.qqqqq"), "File");
    assert_eq!(model_type_group("fv\tarchive-index"), "File");
}

#[test]
fn type_groups_come_from_the_shared_mime_database() {
    let expected = gio::content_type_get_description(
        &gio::content_type_guess(Some(Path::new("notes.json")), None::<&[u8]>).0,
    );

    assert_eq!(model_type_group("fv\tnotes.json"), expected);
}

#[test]
fn repeated_lookups_of_one_suffix_agree() {
    let first = model_type_group("fv\tone.py");
    let second = model_type_group("fv\ttwo.py");

    assert_eq!(first, second);
    assert_ne!(first, "File");
}
