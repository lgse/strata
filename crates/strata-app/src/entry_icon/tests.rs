// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_kit::{
    icons::{catppuccin as ctp_catalog, lucide},
    look::Look,
};

use super::{
    BOX, COG, DISC, DOCUMENTS, EXTENSION_GROUPS, FILE_ARCHIVE, FILE_AUDIO, FILE_SPREADSHEET,
    FILE_TERMINAL, FILE_TYPE, FILENAME_AFFIX_PATTERNS, FILENAME_ICONS, FOLDER, FOLDER_LOCK,
    KEY_ROUND, PICTURES, VIDEOS, X, catppuccin, entry_icon, icon_for_name,
};

fn entry(name: &str, kind: EntryKind, listable: MetadataValue<bool>) -> FileEntry {
    FileEntry {
        location: Location::local(PathBuf::from("/tmp").join(name)),
        thumbnail_path: None,
        display_name: name.to_owned(),
        native_name: name.into(),
        kind,
        is_hidden: name.starts_with('.'),
        listable,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        size: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
    }
}

#[test]
fn resolves_catppuccin_and_lucide_file_type_icons() {
    let cases = [
        ("AGENTS.md", catppuccin::MARKDOWN),
        ("Index.md", catppuccin::MARKDOWN),
        ("flake.nix", catppuccin::NIX),
        ("flake.lock", catppuccin::NIX_LOCK),
        ("justfile", catppuccin::JUST),
        ("skills-lock.json", catppuccin::JSON),
        ("Cargo.toml", catppuccin::CARGO),
        ("lib.rs", catppuccin::RUST),
        ("types.d.ts", catppuccin::TYPESCRIPT_DEF),
        ("app.ts", catppuccin::TYPESCRIPT),
        ("Dockerfile.dev", catppuccin::DOCKER),
        (".env.local", catppuccin::ENV),
        ("tsconfig.app.json", catppuccin::TYPESCRIPT_CONFIG),
        (".bashrc", catppuccin::BASH),
        ("photo.png", PICTURES),
        ("clip.mp4", VIDEOS),
        ("track.flac", FILE_AUDIO),
        ("archive.zip", FILE_ARCHIVE),
        ("archive.tar.gz", FILE_ARCHIVE),
        ("id_ed25519", KEY_ROUND),
        ("cert.pem", KEY_ROUND),
        ("installer.iso", DISC),
        ("table.csv", FILE_SPREADSHEET),
        ("font.ttf", FILE_TYPE),
        ("package.deb", BOX),
        ("bash_history", FILE_TERMINAL),
        ("custom.lock", COG),
        ("Untitled.base", DOCUMENTS),
        ("markdown", DOCUMENTS),
    ];

    for (name, expected) in cases {
        assert_eq!(icon_for_name(name), expected, "name: {name}");
    }
}

#[test]
fn resolves_entry_kinds_for_folders_locks_and_broken_symlinks() {
    let folder = entry(
        "Clippings",
        EntryKind::Directory,
        MetadataValue::Known(true),
    );
    assert_eq!(entry_icon(&folder), FOLDER);

    let locked = entry("root", EntryKind::Directory, MetadataValue::Known(false));
    assert_eq!(entry_icon(&locked), FOLDER_LOCK);

    let broken = entry("dangling", EntryKind::SymbolicLink, MetadataValue::Unknown);
    assert_eq!(entry_icon(&broken), X);

    let markdown = entry("AGENTS.md", EntryKind::File, MetadataValue::Unknown);
    assert_eq!(entry_icon(&markdown), catppuccin::MARKDOWN);
}

#[test]
fn every_mapped_icon_exists_in_all_catalogs() {
    let mut keys: Vec<&'static str> = vec![
        BOX,
        COG,
        DISC,
        DOCUMENTS,
        FILE_ARCHIVE,
        FILE_AUDIO,
        FILE_SPREADSHEET,
        FILE_TERMINAL,
        FILE_TYPE,
        FOLDER,
        FOLDER_LOCK,
        KEY_ROUND,
        PICTURES,
        VIDEOS,
        X,
    ];
    keys.extend(FILENAME_ICONS.iter().map(|(_, icon)| *icon));
    keys.extend(FILENAME_AFFIX_PATTERNS.iter().map(|(_, _, icon)| *icon));
    keys.extend(EXTENSION_GROUPS.iter().map(|(_, icon)| *icon));
    keys.push(catppuccin::MARKDOWN);
    keys.push(catppuccin::NIX);
    keys.push(catppuccin::NIX_LOCK);
    keys.push(catppuccin::JUST);
    keys.push(catppuccin::JSON);
    keys.push(catppuccin::RUST);
    keys.push(catppuccin::CARGO);
    keys.push(catppuccin::TYPESCRIPT_DEF);

    for set in [lucide::SET, ctp_catalog::MOCHA, ctp_catalog::LATTE] {
        let mut look = Look::fallback(true);
        look.icons = set;
        for &key in &keys {
            assert!(
                look.icon_named(key).is_some(),
                "catalog {} missing icon key `{key}`",
                set.name
            );
        }
    }
}

#[test]
fn brand_and_theme_catalogs_color_catppuccin_differently_while_lucide_stays_current_color() {
    let mut brand_dark = Look::fallback(true);
    brand_dark.icons = ctp_catalog::for_background(true);

    let mut brand_light = Look::fallback(false);
    brand_light.icons = ctp_catalog::for_background(false);

    let mut themed = Look::fallback(true);
    themed.icons = lucide::SET;

    let md_dark = brand_dark
        .icon_named(icon_for_name("AGENTS.md"))
        .expect("markdown in mocha");
    let md_light = brand_light
        .icon_named(icon_for_name("AGENTS.md"))
        .expect("markdown in latte");
    let md_theme = themed
        .icon_named(icon_for_name("AGENTS.md"))
        .expect("markdown in lucide");

    assert!(
        !md_dark.body.contains("currentColor"),
        "Brand dark Catppuccin icon should embed Mocha palette colors"
    );
    assert!(
        !md_light.body.contains("currentColor"),
        "Brand light Catppuccin icon should embed Latte palette colors"
    );
    assert_ne!(
        md_dark.body, md_light.body,
        "Mocha and Latte flavors must resolve distinct palettes"
    );
    assert!(
        md_theme.body.contains("currentColor"),
        "Theme-colored Catppuccin icon must use currentColor so accent tints it"
    );

    for look in [&brand_dark, &brand_light, &themed] {
        let folder = look
            .icon_named(FOLDER)
            .expect("folder icon in every catalog");
        assert!(
            folder.body.contains("currentColor"),
            "Lucide folder icon must always take theme accent via currentColor"
        );
    }
}
