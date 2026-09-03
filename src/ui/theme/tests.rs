// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;

use super::{
    Preferences, Theme, azure_tokens, blend, builtins, is_omarchy_theme_event,
    merge_builtin_and_custom_themes, slugify, sort_preferences, source_palette_from_quattro,
    title_case_slug, tokens_from_quattro, validate_tokens,
};
use crate::{
    model::{SortDirection, SortKey, ViewPreferences},
    services::Channel,
};

#[test]
fn bundled_catalog_is_valid_unique_and_alphabetical() {
    let themes = builtins();
    assert_eq!(themes.len(), 95);

    let mut ids = HashSet::new();
    let mut previous_name = String::new();
    for theme in &themes {
        assert!(
            ids.insert(theme.id.as_str()),
            "bundled theme IDs must be unique"
        );
        assert!(
            validate_tokens(&theme.tokens).is_ok(),
            "{} must contain valid theme tokens",
            theme.tokens.name
        );
        let name = theme.tokens.name.to_lowercase();
        assert!(previous_name <= name, "bundled themes must be alphabetical");
        previous_name = name;
    }

    for removed in [
        "apprentice",
        "brogrammer",
        "codeschool",
        "everforest-dark-medium",
        "everforest-light-soft",
        "gruvbox-dark-medium",
        "gruvbox-dark-soft",
        "gruvbox-light-medium",
        "gruvbox-light-soft",
        "jellybeans",
        "shades-of-purple",
        "xcode-dusk",
    ] {
        assert!(!ids.contains(removed), "{removed} should not be bundled");
    }

    let theme_0x96f = themes
        .iter()
        .find(|theme| theme.id == "0x96f")
        .expect("0x96f should be bundled");
    assert_eq!(theme_0x96f.tokens.accent, "#a093e2");
    assert_eq!(
        themes
            .iter()
            .find(|theme| theme.id == "everforest-light-medium")
            .map(|theme| theme.tokens.name.as_str()),
        Some("Everforest Light (Soft)")
    );
    assert_eq!(
        themes
            .iter()
            .find(|theme| theme.id == "gruvbox-dark-hard")
            .map(|theme| theme.tokens.name.as_str()),
        Some("Gruvbox Dark")
    );
    assert_eq!(
        themes
            .iter()
            .find(|theme| theme.id == "gruvbox-light-hard")
            .map(|theme| theme.tokens.name.as_str()),
        Some("Gruvbox Light")
    );
}

#[test]
fn custom_themes_replace_bundled_themes_with_the_same_id() {
    let builtin = Theme {
        id: "dracula".to_owned(),
        tokens: azure_tokens(),
        custom: false,
    };
    let mut custom = builtin.clone();
    custom.tokens.name = "My Dracula".to_owned();
    custom.custom = true;

    let themes = merge_builtin_and_custom_themes(vec![builtin], vec![custom]);

    assert_eq!(themes.len(), 1);
    assert!(themes[0].custom);
    assert_eq!(themes[0].tokens.name, "My Dracula");
}

#[test]
fn names_become_safe_config_file_slugs() {
    assert_eq!(slugify("  Rosé / Pine!  "), "ros-pine");
    assert_eq!(slugify("Ocean  Blue"), "ocean-blue");
}

#[test]
fn omarchy_slugs_become_display_names() {
    assert_eq!(title_case_slug("tokyo-night"), "Tokyo Night");
}

#[test]
fn colors_can_be_blended_into_semantic_tokens() {
    assert_eq!(blend("#000000", "#ffffff", 0.5), "#808080");
}

#[test]
fn quattro_colors_map_to_strata_tokens() {
    let theme = tokens_from_quattro(
        "azure-glow",
        r##"
background = "#0a0f1a"
foreground = "#a8dfff"
accent = "#00aaff"
selection = "#a8dfff"
color8 = "#123247"
"##,
    )
    .expect("valid Quattro colors should map");

    assert_eq!(theme.name, "Azure Glow");
    assert_eq!(theme.background, "#0d1b2a");
    assert_eq!(theme.accent, "#00aaff");
    assert_eq!(theme.border, "#487089");
}

#[test]
fn quattro_syntax_colors_remain_theme_native() {
    let palette = source_palette_from_quattro(
        r##"
blue = "#111111"
cyan = "#222222"
green = "#333333"
yellow = "#444444"
orange = "#555555"
magenta = "#666666"
"##,
    )
    .expect("complete Quattro syntax palette");

    assert_eq!(palette.statement, "#666666");
    assert_eq!(palette.string, "#333333");
    assert_eq!(palette.constant, "#555555");
    assert_eq!(palette.type_color, "#222222");
    assert_eq!(palette.preprocessor, "#444444");
}

#[test]
fn legacy_palette_without_quattro_semantics_is_not_detected() {
    assert!(tokens_from_quattro("legacy", "color4 = \"#00aaff\"").is_none());
}

#[test]
fn omarchy_monitor_ignores_unrelated_state_changes() {
    assert!(is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/theme"
    )));
    assert!(is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/theme.name"
    )));
    assert!(!is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/next-theme"
    )));
    assert!(!is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/background"
    )));
}

#[test]
fn legacy_preferences_enable_single_click_previews_by_default() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
"#,
    )
    .expect("legacy preferences should remain valid");

    assert!(preferences.folder_peeking);
    assert!(preferences.single_click_previews);
    assert!(preferences.render_documents_by_default);
    assert!(!preferences.search_open_files_directly);
    assert!(!preferences.reduce_motion);
    assert_eq!(preferences.browser_mode, "columns");
    assert_eq!(preferences.browser_density, "compact");
    assert_eq!(sort_preferences(&preferences), ViewPreferences::default());
}

#[test]
fn rendered_document_preference_can_be_disabled_and_round_trips() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
render_documents_by_default = false
"#,
    )
    .expect("preferences should be valid");
    assert!(!preferences.render_documents_by_default);

    let serialized = toml::to_string(&preferences).expect("preferences should serialize");
    let restored: Preferences =
        toml::from_str(&serialized).expect("preferences should deserialize");
    assert!(!restored.render_documents_by_default);
}

#[test]
fn view_preferences_round_trip_all_supported_sorting_values() {
    for (key, stored_key) in [
        (SortKey::Name, "name"),
        (SortKey::Size, "size"),
        (SortKey::Modified, "modified"),
        (SortKey::Type, "type"),
    ] {
        for (direction, stored_direction) in [
            (SortDirection::Ascending, "ascending"),
            (SortDirection::Descending, "descending"),
        ] {
            let preferences = Preferences {
                show_hidden: true,
                folders_first: false,
                sort_key: stored_key.to_owned(),
                sort_direction: stored_direction.to_owned(),
                ..Preferences::default()
            };
            let serialized = toml::to_string(&preferences).expect("preferences should serialize");
            let restored: Preferences =
                toml::from_str(&serialized).expect("preferences should deserialize");
            assert_eq!(
                sort_preferences(&restored),
                ViewPreferences {
                    show_hidden: true,
                    folders_first: false,
                    sort_key: key,
                    sort_direction: direction,
                }
            );
        }
    }
}

#[test]
fn invalid_sorting_preferences_fall_back_as_a_pair() {
    for (key, direction) in [("unknown", "descending"), ("size", "sideways")] {
        let preferences = Preferences {
            show_hidden: true,
            folders_first: false,
            sort_key: key.to_owned(),
            sort_direction: direction.to_owned(),
            ..Preferences::default()
        };
        assert_eq!(
            sort_preferences(&preferences),
            ViewPreferences {
                show_hidden: true,
                folders_first: false,
                ..ViewPreferences::default()
            }
        );
    }
}

#[test]
fn general_preferences_round_trip() {
    let preferences = Preferences {
        folder_peeking: false,
        single_click_previews: false,
        search_open_files_directly: true,
        reduce_motion: true,
        ..Preferences::default()
    };

    let serialized = toml::to_string(&preferences).expect("preferences should serialize");
    let restored: Preferences =
        toml::from_str(&serialized).expect("preferences should deserialize");

    assert!(!restored.folder_peeking);
    assert!(!restored.single_click_previews);
    assert!(restored.search_open_files_directly);
    assert!(restored.reduce_motion);
}

#[test]
fn release_channel_defaults_to_stable() {
    let preferences = Preferences::default();
    assert_eq!(preferences.release_channel, "stable");
    assert_eq!(
        Channel::parse(&preferences.release_channel),
        Channel::Stable
    );
}

#[test]
fn preview_release_channel_round_trips_through_toml() {
    let preferences = Preferences {
        release_channel: "preview".to_owned(),
        ..Preferences::default()
    };
    let serialized = toml::to_string(&preferences).expect("preferences should serialize");
    let restored: Preferences =
        toml::from_str(&serialized).expect("preferences should deserialize");
    assert_eq!(restored.release_channel, "preview");
    assert_eq!(Channel::parse(&restored.release_channel), Channel::Preview);
}

#[test]
fn nightly_release_channel_round_trips_through_toml() {
    let preferences = Preferences {
        release_channel: "nightly".to_owned(),
        ..Preferences::default()
    };
    let serialized = toml::to_string(&preferences).expect("preferences should serialize");
    let restored: Preferences =
        toml::from_str(&serialized).expect("preferences should deserialize");
    assert_eq!(restored.release_channel, "nightly");
    assert_eq!(Channel::parse(&restored.release_channel), Channel::Nightly);
}

#[test]
fn unknown_release_channel_value_parses_to_stable() {
    let preferences = Preferences {
        release_channel: "experimental".to_owned(),
        ..Preferences::default()
    };
    assert_eq!(
        Channel::parse(&preferences.release_channel),
        Channel::Stable
    );
}

#[test]
fn legacy_preferences_without_release_channel_default_to_stable() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
"#,
    )
    .expect("legacy preferences without release_channel should remain valid");

    assert_eq!(preferences.release_channel, "stable");
    assert_eq!(
        Channel::parse(&preferences.release_channel),
        Channel::Stable
    );
}
