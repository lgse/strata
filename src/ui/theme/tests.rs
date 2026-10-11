// SPDX-License-Identifier: MIT

mod syntax;
mod text_size;

use std::collections::HashSet;

use super::{
    Theme, ThemeManager, ThemeTokens, azure_tokens, blend, builtins, canonical_color, color_to_hex,
    is_omarchy_theme_event, merge_builtin_and_custom_themes, slugify, source_palette_from_quattro,
    source_style_scheme_xml, themes_directory, title_case_slug, tokens_from_quattro,
    validate_tokens,
};
use crate::test_support::{gtk_test, texture_has_visible_pixels};

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
fn saving_a_custom_theme_never_overwrites_an_existing_theme_file() {
    gtk_test(
        "ui::theme::tests::saving_a_custom_theme_never_overwrites_an_existing_theme_file",
        || {
            let directory = themes_directory();
            let dotfiles = directory
                .parent()
                .expect("config directory")
                .join("dotfiles");
            std::fs::create_dir_all(&directory).expect("themes directory");
            std::fs::create_dir_all(&dotfiles).expect("dotfiles directory");
            let mut tokens = azure_tokens();
            tokens.name = "Ocean Blue".to_owned();
            let valid = toml::to_string_pretty(&tokens).expect("theme file");
            std::fs::write(directory.join("ocean-blue.toml"), "not a theme").expect("broken theme");
            std::fs::write(dotfiles.join("broken.toml"), "name = [").expect("broken dotfile");
            std::os::unix::fs::symlink(
                dotfiles.join("broken.toml"),
                directory.join("ocean-blue-2.toml"),
            )
            .expect("link to a broken dotfile");
            std::os::unix::fs::symlink(
                dotfiles.join("missing.toml"),
                directory.join("ocean-blue-3.toml"),
            )
            .expect("dangling link");
            let manager = ThemeManager::shared();
            std::fs::write(dotfiles.join("valid.toml"), &valid).expect("valid dotfile");
            std::os::unix::fs::symlink(
                dotfiles.join("valid.toml"),
                directory.join("ocean-blue-4.toml"),
            )
            .expect("link added after startup");

            let id = manager.save_custom_theme(tokens).expect("saved theme");

            assert_eq!(id, "ocean-blue-5");
            assert!(directory.join("ocean-blue-5.toml").is_file());
            assert_eq!(
                std::fs::read_to_string(directory.join("ocean-blue.toml")).expect("broken theme"),
                "not a theme"
            );
            for (link, target) in [
                ("ocean-blue-2.toml", "broken.toml"),
                ("ocean-blue-3.toml", "missing.toml"),
                ("ocean-blue-4.toml", "valid.toml"),
            ] {
                assert_eq!(
                    std::fs::read_link(directory.join(link)).expect("theme link"),
                    dotfiles.join(target)
                );
            }
            assert_eq!(
                std::fs::read_to_string(dotfiles.join("broken.toml")).expect("broken dotfile"),
                "name = ["
            );
            assert!(!dotfiles.join("missing.toml").exists());
            assert_eq!(
                std::fs::read_to_string(dotfiles.join("valid.toml")).expect("valid dotfile"),
                valid
            );
        },
    );
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
    for (value, css, opaque) in [
        ("rgb(153,193,241)", Some("#99c1f1"), "#99c1f1"),
        ("#fff", Some("#ffffff"), "#ffffff"),
        ("#81A1C1", Some("#81a1c1"), "#81a1c1"),
        ("rebeccapurple", Some("#663399"), "#663399"),
        ("#000aaafff", Some("#00aaff"), "#00aaff"),
        ("#0000aaaaffff", Some("#00aaff"), "#00aaff"),
        ("#0000aaaaffff8888", Some("#00aaff88"), "#00aaff"),
        ("rgba(0, 170, 255, 0.5)", Some("#00aaff80"), "#00aaff"),
        ("0x7aa2f7", None, "0x7aa2f7"),
    ] {
        assert_eq!(canonical_color(value).as_deref(), css, "{value}");
        assert_eq!(color_to_hex(value), opaque, "{value}");
    }
}

#[test]
fn custom_theme_colors_load_as_css_hex_without_rewriting_the_file() {
    gtk_test(
        "ui::theme::tests::custom_theme_colors_load_as_css_hex_without_rewriting_the_file",
        || {
            use gtk::prelude::*;
            let directory = themes_directory();
            std::fs::create_dir_all(&directory).expect("themes directory");
            let mut written = azure_tokens();
            written.name = "Deep Hex".to_owned();
            written.accent = "#000aaafff".to_owned();
            written.highlight = "#0000aaaaffff8888".to_owned();
            written.syntax_string = Some("#333333333".to_owned());
            let source = toml::to_string_pretty(&written).expect("theme file");
            let path = directory.join("deep-hex.toml");
            std::fs::write(&path, &source).expect("custom theme");

            let manager = ThemeManager::shared();
            let loaded = manager
                .themes()
                .into_iter()
                .find(|theme| theme.id == "deep-hex")
                .expect("custom theme with GTK-only hex colors loads")
                .tokens;
            assert_eq!(loaded.accent, "#00aaff");
            assert_eq!(loaded.highlight, "#00aaff88");
            assert_eq!(loaded.syntax_string.as_deref(), Some("#333333"));
            assert_eq!(loaded.background, written.background);

            let window = gtk::Window::new();
            manager.select_theme("deep-hex");
            for (name, expected) in [
                ("strata_accent", "#00aaff"),
                ("strata_highlight", "#00aaff88"),
            ] {
                #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                let applied = window.style_context().lookup_color(name);
                assert_eq!(
                    applied,
                    Some(gtk::gdk::RGBA::parse(expected).expect("canonical color")),
                    "{name}"
                );
            }
            assert_eq!(crate::assets::primary_icon_color(), "#00aaff");
            assert_eq!(
                std::fs::read_to_string(&path).expect("custom theme"),
                source,
                "loading never rewrites the user's theme file"
            );
            window.close();
        },
    );
}

#[test]
fn applied_icon_colors_are_canonical_and_render_visible_strokes() {
    gtk_test(
        "ui::theme::tests::applied_icon_colors_are_canonical_and_render_visible_strokes",
        || {
            use gtk::prelude::*;
            let manager = ThemeManager::shared();
            for (value, expected) in [
                ("#000aaafff", "#00aaff"),
                ("rgb(0, 170, 255)", "#00aaff"),
                ("deepskyblue", "#00bfff"),
                ("#00aaff80", "#00aaff"),
            ] {
                let mut tokens = azure_tokens();
                tokens.accent = value.to_owned();
                tokens.text = value.to_owned();
                tokens.danger = value.to_owned();
                manager.apply_tokens(&tokens, None);
                assert_eq!(crate::assets::primary_icon_color(), expected, "{value}");
                for (kind, image) in [
                    (
                        "primary",
                        crate::assets::primary_icon(crate::assets::icons::COG, 18),
                    ),
                    (
                        "text",
                        crate::assets::text_icon(crate::assets::icons::COG, 18),
                    ),
                    (
                        "danger",
                        crate::assets::danger_icon(crate::assets::icons::COG, 18),
                    ),
                ] {
                    let texture = image
                        .paintable()
                        .expect("icon paintable")
                        .downcast::<gtk::gdk::Texture>()
                        .expect("icon texture");
                    assert!(
                        texture_has_visible_pixels(&texture),
                        "{value} {kind} icon has a visible stroke"
                    );
                }
            }
        },
    );
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
        super::OmarchyVariant::Original,
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
        let tokens = tokens_from_quattro("azure-glow", &source, super::OmarchyVariant::Original)
            .expect("partial Quattro palette");
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
fn quattro_syntax_palette_treats_unparsable_colors_as_missing() {
    let base = [
        ("blue", "#111111"),
        ("cyan", "#222222"),
        ("green", "#333333"),
        ("yellow", "#444444"),
        ("orange", "#555555"),
        ("magenta", "#666666"),
    ];
    let source_with = |key: &str, value: &str| {
        base.iter()
            .map(|(name, color)| {
                let color = if *name == key { value } else { color };
                format!("{name} = \"{color}\"\n")
            })
            .collect::<String>()
    };
    type Field = fn(&super::SourcePalette) -> &str;
    let statement: Field = |palette| &palette.statement;
    let string: Field = |palette| &palette.string;
    let constant: Field = |palette| &palette.constant;
    let type_color: Field = |palette| &palette.type_color;
    for (key, value, expected) in [
        ("green", "invalid", None),
        ("yellow", "invalid", None),
        ("magenta", "0x666666", Some((statement, "#111111"))),
        ("orange", "notacolor", Some((constant, "#444444"))),
        ("cyan", "", Some((type_color, "#111111"))),
        ("green", "#333333333", Some((string, "#333333"))),
        (
            "magenta",
            "rgba(102,102,102,0.5)",
            Some((statement, "#66666680")),
        ),
    ] {
        let palette = source_palette_from_quattro(&source_with(key, value));
        match expected {
            None => assert!(palette.is_none(), "{key} = {value:?}"),
            Some((field, color)) => assert_eq!(
                palette.as_ref().map(field),
                Some(color),
                "{key} = {value:?}"
            ),
        }
    }

    let source = format!(
        "background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\n{}",
        source_with("orange", "0x555555").replace("#666666", "notacolor")
    );
    let tokens = tokens_from_quattro("azure-glow", &source, super::OmarchyVariant::Original)
        .expect("required Quattro colors are valid");
    let xml = source_style_scheme_xml(&tokens, source_palette_from_quattro(&source).as_ref());
    let values = scheme_color_values(&xml);
    assert_eq!(values.len(), 12);
    for value in values {
        assert!(
            value.starts_with('#') && value.len() == 7,
            "scheme colors must be canonical #rrggbb, got {value}"
        );
    }
}

#[test]
fn legacy_palette_without_quattro_semantics_is_not_detected() {
    assert!(
        tokens_from_quattro(
            "legacy",
            "color4 = \"#00aaff\"",
            super::OmarchyVariant::Original
        )
        .is_none()
    );
}

#[test]
fn omarchy_monitor_ignores_unrelated_state_changes() {
    let state = super::omarchy_state_dir();
    for path in [
        state.clone(),
        state.join("theme"),
        state.join("theme.name"),
        state.join("theme/colors.toml"),
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

#[test]
fn appearance_changes_keep_an_active_preview_until_it_is_cancelled() {
    gtk_test(
        "ui::theme::tests::appearance_changes_keep_an_active_preview_until_it_is_cancelled",
        || {
            use crate::ui::preferences::{PreferenceManager, TextSize};

            let manager = super::ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");
            let saved = manager.active_model_palette();
            let mut tokens = manager.starter_tokens();
            tokens.accent = "#13579b".to_owned();
            manager.preview(&tokens);
            assert_eq!(manager.active_model_palette().accent, 0x13579b);

            let preferences = PreferenceManager::shared();
            let original = preferences.text_size();
            let changed = if original.root_font_px() == 20 {
                18
            } else {
                20
            };
            preferences.set_text_size(TextSize::new(changed));
            assert_eq!(
                manager.active_model_palette().accent,
                0x13579b,
                "a text size change keeps the unsaved preview applied"
            );
            assert!(manager.is_previewing());

            manager.cancel_preview();
            assert!(!manager.is_previewing());
            assert_eq!(manager.active_model_palette(), saved);
            preferences.set_text_size(original);
            assert_eq!(manager.active_model_palette(), saved);
        },
    );
}
