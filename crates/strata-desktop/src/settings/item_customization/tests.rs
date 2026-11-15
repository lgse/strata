// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use strata_core::model::{FolderColor, FolderColorValue};

use super::{custom_emoji, is_icon_choice};
use crate::settings::Preferences;

#[test]
fn an_icon_is_kept_only_while_an_item_may_wear_it() {
    let path = Path::new("/home/user/Projects");
    let mut preferences = Preferences::default();
    for (icon, kept) in [
        ("strata-file-code", true),
        ("emoji:🚀", true),
        ("emoji:", false),
        ("folder-from-system-theme", false),
    ] {
        preferences.set_item_icon(path, Some(icon));
        assert_eq!(preferences.item_icon(path).is_some(), kept, "{icon}");
    }
    // A name the file holds that no item may wear any more reads as no icon.
    preferences.custom_icons.insert(
        path.to_string_lossy().into_owned(),
        "strata-gone".to_owned(),
    );
    assert_eq!(preferences.item_icon(path), None);
}

#[test]
fn colours_save_for_every_path_and_clearing_forgets_both() {
    let paths = [PathBuf::from("/a"), PathBuf::from("/b")];
    let mut preferences = Preferences::default();
    let red = FolderColorValue::Preset(FolderColor::Red);
    preferences.set_item_colors(&paths, Some(&red));
    preferences.set_item_icon(&paths[0], Some("strata-pin"));
    assert_eq!(
        preferences.folder_colors.get("/b").map(String::as_str),
        Some("red")
    );
    assert!(preferences.is_item_customized(&paths[0]));

    preferences.clear_item_customization(&paths[0]);
    assert!(!preferences.is_item_customized(&paths[0]));
    assert_eq!(preferences.item_color(&paths[1]), Some(red));

    let custom = FolderColorValue::Custom("#34D399".to_owned());
    preferences.set_item_colors(&paths[1..], Some(&custom));
    assert_eq!(
        preferences.folder_colors.get("/b").map(String::as_str),
        Some("#34d399")
    );
    preferences.set_item_colors(&paths[1..], None);
    assert_eq!(preferences.item_color(&paths[1]), None);
}

#[test]
fn a_saved_emoji_is_bounded_and_safe_to_draw() {
    assert_eq!(custom_emoji("emoji:👨‍👩‍👧‍👦"), Some("👨‍👩‍👧‍👦"));
    assert_eq!(custom_emoji("emoji:\n"), None);
    assert_eq!(custom_emoji(&format!("emoji:{}", "x".repeat(65))), None);
    assert!(!is_icon_choice("strata-folder"));
}
