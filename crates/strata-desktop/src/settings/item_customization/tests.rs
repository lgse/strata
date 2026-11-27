// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use rustix::fs::FileType;
use strata_core::model::{FolderColor, FolderColorValue};

use super::{TrashedCustomizations, custom_emoji, is_icon_choice};
use crate::{
    services::{Evidence, IdentityTraits, InodeNumbers, ItemIdentity},
    settings::{Preferences, trashed::TrashedValues},
};

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

fn dir_identity(inode: u64) -> ItemIdentity {
    ItemIdentity {
        evidence: Evidence {
            device: 1,
            inode,
            kind: FileType::Directory,
            birth: Some((inode as i64, 0)),
            handle: None,
            attributes: None,
        },
        traits: IdentityTraits {
            numbers: InodeNumbers::Kept,
            ..IdentityTraits::default()
        },
    }
}

fn seed(preferences: &mut Preferences, path: &str, color: FolderColor, icon: Option<&str>) {
    let path = PathBuf::from(path);
    preferences.set_item_colors(
        std::slice::from_ref(&path),
        Some(&FolderColorValue::Preset(color)),
    );
    preferences.set_item_icon(&path, icon);
}

#[test]
fn relocation_rebases_subtrees_clears_stale_targets_preserves_merges_and_forgets_remote() {
    let mut preferences = Preferences::default();
    seed(
        &mut preferences,
        "/w/docs",
        FolderColor::Red,
        Some("strata-file-code"),
    );
    seed(&mut preferences, "/w/docs/inner", FolderColor::Blue, None);
    seed(&mut preferences, "/w/docs2", FolderColor::Green, None);
    seed(
        &mut preferences,
        "/w/dest/docs",
        FolderColor::Yellow,
        Some("strata-pin"),
    );
    seed(
        &mut preferences,
        "/w/dest/docs/old",
        FolderColor::Orange,
        None,
    );

    assert!(preferences.relocate_item_customizations(
        Path::new("/w/docs"),
        Some(Path::new("/w/dest/docs")),
        false,
    ));
    assert!(!preferences.relocate_item_customizations(
        Path::new("/w/docs"),
        Some(Path::new("/w/dest/docs")),
        false,
    ));
    assert!(!preferences.is_item_customized(Path::new("/w/docs")));
    assert!(!preferences.is_item_customized(Path::new("/w/dest/docs/old")));
    assert_eq!(
        preferences.item_color(Path::new("/w/dest/docs")),
        Some(FolderColorValue::Preset(FolderColor::Red))
    );
    assert_eq!(
        preferences.item_icon(Path::new("/w/dest/docs")),
        Some("strata-file-code")
    );
    assert_eq!(
        preferences.item_color(Path::new("/w/dest/docs/inner")),
        Some(FolderColorValue::Preset(FolderColor::Blue))
    );
    assert_eq!(
        preferences.item_color(Path::new("/w/docs2")),
        Some(FolderColorValue::Preset(FolderColor::Green))
    );

    // A merged move keeps the destination folder's own customization and carries descendants.
    seed(
        &mut preferences,
        "/w/incoming",
        FolderColor::Purple,
        Some("emoji:🚀"),
    );
    seed(
        &mut preferences,
        "/w/incoming/child",
        FolderColor::Gray,
        None,
    );
    assert!(preferences.relocate_item_customizations(
        Path::new("/w/incoming"),
        Some(Path::new("/w/dest/docs")),
        true,
    ));
    assert_eq!(
        preferences.item_color(Path::new("/w/dest/docs")),
        Some(FolderColorValue::Preset(FolderColor::Red))
    );
    assert_eq!(
        preferences.item_icon(Path::new("/w/dest/docs")),
        Some("strata-file-code")
    );
    assert_eq!(
        preferences.item_color(Path::new("/w/dest/docs/child")),
        Some(FolderColorValue::Preset(FolderColor::Gray))
    );

    // Moving off local storage (`to: None`) forgets the subtree.
    assert!(preferences.relocate_item_customizations(Path::new("/w/dest/docs"), None, false));
    assert!(!preferences.is_item_customized(Path::new("/w/dest/docs")));
    assert!(!preferences.is_item_customized(Path::new("/w/dest/docs/child")));
}

#[test]
fn trash_keeps_customizations_by_identity_and_permanent_delete_drops_them() {
    let mut preferences = Preferences::default();
    let mut trashed = TrashedValues::<TrashedCustomizations>::default();
    seed(
        &mut preferences,
        "/w/notes",
        FolderColor::Red,
        Some("emoji:🎵"),
    );
    seed(&mut preferences, "/w/notes/sub", FolderColor::Blue, None);
    seed(
        &mut preferences,
        "/w/gone",
        FolderColor::Green,
        Some("strata-pin"),
    );

    let notes_id = dir_identity(10);
    assert!(preferences.forget_item_customizations(
        Path::new("/w/notes"),
        Some(notes_id),
        &mut trashed,
    ));
    assert!(!preferences.is_item_customized(Path::new("/w/notes")));

    // Another item restored at the same path gets nothing; the original item gets its keys back.
    assert!(!preferences.restore_item_customizations(
        Path::new("/w/notes"),
        dir_identity(99),
        &mut trashed,
    ));
    assert!(
        preferences.restore_item_customizations(Path::new("/w/notes"), notes_id, &mut trashed,)
    );
    assert_eq!(
        preferences.item_icon(Path::new("/w/notes")),
        Some("emoji:🎵")
    );
    assert_eq!(
        preferences.item_color(Path::new("/w/notes/sub")),
        Some(FolderColorValue::Preset(FolderColor::Blue))
    );

    // Permanent delete (`None`) keeps nothing in `trashed`.
    let gone_id = dir_identity(20);
    assert!(preferences.forget_item_customizations(Path::new("/w/gone"), None, &mut trashed));
    assert!(!preferences.restore_item_customizations(Path::new("/w/gone"), gone_id, &mut trashed,));
}
