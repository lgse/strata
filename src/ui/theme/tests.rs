// SPDX-License-Identifier: MIT

mod syntax;
mod text_size;

use std::collections::HashSet;

use super::{
    Theme, ThemeTokens, azure_tokens, blend, builtins, color_to_hex, is_omarchy_theme_event,
    merge_builtin_and_custom_themes, slugify, source_palette_from_quattro, source_style_scheme_xml,
    title_case_slug, tokens_from_quattro, validate_tokens,
};
use crate::test_support::gtk_test;

#[test]
fn bundled_catalog_is_valid_unique_and_alphabetical() {
    let themes = builtins();

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
    assert_eq!(blend("rgb(0,0,0)", "rgb(255,255,255)", 0.5), "#808080");
    assert_eq!(blend("#000", "#fff", 0.5), "#808080");
    assert_eq!(blend("black", "white", 0.5), "#808080");
}

#[test]
fn gtk_color_formats_canonicalize_for_persistence_and_scheme_xml() {
    assert_eq!(color_to_hex("rgb(153,193,241)"), "#99c1f1");
    assert_eq!(color_to_hex("#fff"), "#ffffff");
    assert_eq!(color_to_hex("rebeccapurple"), "#663399");
}

fn scheme_color_values(xml: &str) -> Vec<&str> {
    xml.lines()
        .filter_map(|line| {
            let start = line.find("value=\"")? + 7;
            let rest = line.get(start..)?;
            let end = rest.find('"')?;
            Some(&rest[..end])
        })
        .collect()
}

#[test]
fn source_style_scheme_xml_canonicalizes_rgb_tokens_for_gtksourceview() {
    gtk_test(
        "ui::theme::tests::source_style_scheme_xml_canonicalizes_rgb_tokens_for_gtksourceview",
        || {
            let tokens = ThemeTokens {
                name: "Picker".to_owned(),
                background: "rgb(255,255,255)".to_owned(),
                surface: "rgb(245,245,245)".to_owned(),
                text: "rgb(30,29,31)".to_owned(),
                accent: "rgb(153,193,241)".to_owned(),
                danger: "rgb(229,72,77)".to_owned(),
                muted: "rgb(200,200,200)".to_owned(),
                highlight: "rgb(36,77,104)".to_owned(),
                border: "rgb(49,91,117)".to_owned(),
                dim_text: "rgb(111,141,163)".to_owned(),
                syntax_keyword: None,
                syntax_string: None,
                syntax_constant: None,
                syntax_type: None,
                syntax_preprocessor: None,
            };
            let xml = source_style_scheme_xml(&tokens, None);
            let values = scheme_color_values(&xml);
            assert_eq!(values.len(), 12);
            for value in &values {
                assert!(
                    value.starts_with('#') && value.len() == 7,
                    "scheme colors must be canonical #rrggbb, got {value}"
                );
            }
            assert!(
                !xml.contains("rgb("),
                "scheme XML must not emit rgb() colour tags"
            );

            let directory = tempfile::tempdir().expect("scheme directory");
            std::fs::write(directory.path().join("strata-current.xml"), xml.as_bytes())
                .expect("write scheme");
            let manager = sourceview5::StyleSchemeManager::new();
            manager.set_search_path(&[directory.path().to_str().expect("utf-8 scheme path")]);
            manager.force_rescan();
            let scheme = manager
                .scheme("strata-current")
                .expect("GtkSourceView should load a #rrggbb scheme");
            let statement = scheme
                .style("def:statement")
                .expect("def:statement should resolve");
            assert_eq!(statement.foreground().as_deref(), Some("#99c1f1"));
        },
    );
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
    let derived = super::resolved_source_palette(&theme, None);
    for (source, expected) in [
        ("color2 = \"#00ff00\"", "#00ff00"),
        ("color2 = \"#00ff00\"\ngreen = \"#009900\"", "#009900"),
        ("green = \"invalid\"", derived.string.as_str()),
    ] {
        let source = format!(
            "background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\nselection = \"#a8dfff\"\ncolor8 = \"#123247\"\n{source}"
        );
        let tokens = tokens_from_quattro("azure-glow", &source).expect("partial Quattro palette");
        let palette = super::resolved_source_palette(&tokens, None);
        assert_eq!(palette.string, expected);
        assert_eq!(palette.statement, derived.statement);
    }
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
    let state = super::omarchy_state_dir();
    for path in [
        state.clone(),
        state.join("theme"),
        state.join("theme.name"),
        state.parent().expect("Omarchy state parent").to_path_buf(),
    ] {
        assert!(is_omarchy_theme_event(&gtk::gio::File::for_path(path)));
    }
    for path in [
        state.join("next-theme"),
        state.join("background"),
        gtk::glib::home_dir().join("theme"),
    ] {
        assert!(!is_omarchy_theme_event(&gtk::gio::File::for_path(path)));
    }
}
