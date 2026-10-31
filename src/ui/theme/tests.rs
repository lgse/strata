// SPDX-License-Identifier: MIT

mod density;
mod syntax;
mod text_size;

use std::{cell::Cell, collections::HashSet, path::Path, rc::Rc};

use crate::ui::preferences::{CornerRoundness, PreferenceManager};

use super::{
    PINNED_GTK_THEME, Theme, ThemeManager, ThemeTokens, azure_tokens, blend, builtins,
    canonical_color, color_to_hex, custom_theme_id, is_omarchy_theme_event,
    merge_builtin_and_custom_themes, omarchy::contrast, slugify, source_palette_from_quattro,
    source_style_scheme_xml, themes_directory, title_case_slug, tokens_from_quattro,
    validate_tokens,
};
use crate::test_support::{gtk_test, gtk_test_with_env, texture_has_visible_pixels};
use gtk::prelude::*;

fn save_custom_theme(manager: &Rc<ThemeManager>, tokens: ThemeTokens) -> std::io::Result<String> {
    let saved = Rc::new(std::cell::RefCell::new(None));
    let done = saved.clone();
    manager.save_custom_theme(tokens, move |result| {
        done.replace(Some(result));
    });
    crate::storage::writer::settle();
    saved.take().expect("theme save finished")
}

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

type TextColor = (&'static str, fn(&ThemeTokens) -> &str);
type ContrastFloor = (&'static str, fn(&ThemeTokens) -> &str, f64);

#[test]
fn bundled_themes_keep_text_readable() {
    let colors: [TextColor; 2] = [
        ("text", |tokens| &tokens.text),
        ("dim text", |tokens| &tokens.dim_text),
    ];
    let floors: [ContrastFloor; 2] = [
        ("background", |tokens| &tokens.background, 4.5),
        ("surface", |tokens| &tokens.surface, 3.0),
    ];
    let themes = builtins();
    assert!(!themes.is_empty());
    let mut failures = Vec::new();
    for theme in &themes {
        for (name, color) in colors {
            for (role, background, floor) in floors {
                let ratio = contrast(color(&theme.tokens), background(&theme.tokens));
                if ratio < floor {
                    failures.push(format!(
                        "{} {name} on {role} is {ratio:.2}:1, below the {floor}:1 floor",
                        theme.id
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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

            let id = save_custom_theme(&manager, tokens).expect("saved theme");

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
fn rewriting_the_selected_custom_theme_applies_without_a_restart() {
    gtk_test(
        "ui::theme::tests::rewriting_the_selected_custom_theme_applies_without_a_restart",
        || {
            let directory = themes_directory();
            std::fs::create_dir_all(&directory).expect("themes directory");
            let mut tokens = azure_tokens();
            tokens.name = "Live Reload".to_owned();
            tokens.accent = "#112233".to_owned();
            let path = directory.join("live-reload.toml");
            let write_theme = |tokens: &ThemeTokens| {
                std::fs::write(&path, toml::to_string_pretty(tokens).expect("theme file"))
                    .expect("written theme");
            };
            write_theme(&tokens);

            let manager = ThemeManager::shared();
            manager.select_theme("live-reload");
            assert_eq!(manager.active_model_palette().accent, 0x11_22_33);

            tokens.accent = "#445566".to_owned();
            write_theme(&tokens);
            wait_for_live_theme(|| manager.active_model_palette().accent == 0x44_55_66);
            assert_eq!(
                manager.current_tokens().expect("selected theme").accent,
                "#445566"
            );

            std::fs::write(&path, "not a theme").expect("broken theme");
            settle_live_theme();
            assert_eq!(manager.active_model_palette().accent, 0x44_55_66);
            assert!(
                manager
                    .themes()
                    .iter()
                    .any(|theme| theme.id == "live-reload"),
                "a broken file keeps the selected theme listed"
            );

            tokens.accent = "#778899".to_owned();
            write_theme(&tokens);
            wait_for_live_theme(|| manager.active_model_palette().accent == 0x77_88_99);
        },
    );
}

#[test]
fn rewriting_a_symlinked_theme_target_applies_without_a_restart() {
    gtk_test(
        "ui::theme::tests::rewriting_a_symlinked_theme_target_applies_without_a_restart",
        || {
            let directory = themes_directory();
            let config = directory.parent().expect("config directory");
            let (dotfiles, generated) = (config.join("dotfiles"), config.join("generated"));
            for path in [&directory, &dotfiles, &generated] {
                std::fs::create_dir_all(path).expect("theme directories");
            }
            let write_theme = |folder: &Path, name: &str, accent: &str| {
                let mut tokens = azure_tokens();
                tokens.name = name.to_owned();
                tokens.accent = accent.to_owned();
                std::fs::write(
                    folder.join(format!("{name}.toml")),
                    toml::to_string_pretty(&tokens).expect("theme file"),
                )
                .expect("written dotfile");
            };
            let link = |folder: &Path, name: &str| {
                std::os::unix::fs::symlink(
                    folder.join(format!("{name}.toml")),
                    directory.join(format!("{name}.toml")),
                )
                .expect("dotfiles link");
            };
            write_theme(&dotfiles, "linked", "#112233");
            link(&dotfiles, "linked");

            let manager = ThemeManager::shared();
            manager.select_theme("linked");
            assert_eq!(manager.active_model_palette().accent, 0x11_22_33);

            // Dangling until the generator first writes it, in a directory not yet watched.
            link(&generated, "later");
            write_theme(&dotfiles, "linked", "#445566");
            wait_for_live_theme(|| manager.active_model_palette().accent == 0x44_55_66);

            write_theme(&generated, "later", "#778899");
            wait_for_live_theme(|| manager.themes().iter().any(|theme| theme.id == "later"));
            manager.select_theme("later");
            assert_eq!(manager.active_model_palette().accent, 0x77_88_99);

            write_theme(&generated, "later", "#aabbcc");
            wait_for_live_theme(|| manager.active_model_palette().accent == 0xaa_bb_cc);
        },
    );
}

#[test]
fn switching_omarchy_symlinks_keeps_direct_and_atomic_palette_edits_live() {
    gtk_test(
        "ui::theme::tests::switching_omarchy_symlinks_keeps_direct_and_atomic_palette_edits_live",
        || {
            let state = super::omarchy_state_dir();
            let themes = gtk::glib::home_dir().join("palettes");
            let write = |folder: &Path, accent: &str, atomic: bool| {
                std::fs::create_dir_all(folder).expect("palette directory");
                let destination = folder.join("colors.toml");
                let path = if atomic {
                    folder.join("colors.new")
                } else {
                    destination.clone()
                };
                std::fs::write(
                    &path,
                    format!(
                        "background = '#112233'\nforeground = '#ddeeff'\naccent = '{accent}'\n"
                    ),
                )
                .expect("palette written");
                if atomic {
                    std::fs::rename(path, destination).expect("atomic palette replacement");
                }
            };
            let switch = |name: &str| {
                let pending = state.join("next-theme");
                std::os::unix::fs::symlink(themes.join(name), &pending).expect("new theme link");
                std::fs::rename(pending, state.join("theme")).expect("atomic theme switch");
            };
            std::fs::create_dir_all(&state).expect("Omarchy state");
            std::fs::write(state.join("theme.name"), "fixture").expect("theme name");
            write(&themes.join("a"), "#112244", false);
            write(&themes.join("b"), "#445566", false);
            switch("a");
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(true);
            assert_eq!(manager.active_model_palette().accent, 0x11_22_44);

            for (name, initial, edited, color) in [
                ("b", 0x44_55_66, "#778899", 0x77_88_99),
                ("a", 0x11_22_44, "#aabbcc", 0xaa_bb_cc),
            ] {
                switch(name);
                wait_for_live_theme(|| manager.active_model_palette().accent == initial);
                write(&themes.join(name), edited, false);
                wait_for_live_theme(|| manager.active_model_palette().accent == color);
                write(&themes.join(name), "#abcdef", true);
                wait_for_live_theme(|| manager.active_model_palette().accent == 0xab_cd_ef);
            }
        },
    );
}

fn wait_for_live_theme(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "live theme refresh timed out"
        );
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn has_color(label: &gtk::Label, expected: &gtk::gdk::RGBA) -> bool {
    let actual = label.color();
    (actual.red() - expected.red()).abs() < 0.01
        && (actual.green() - expected.green()).abs() < 0.01
        && (actual.blue() - expected.blue()).abs() < 0.01
}

/// Runs the main loop past the refresh debounce so a missing change is a real absence.
fn settle_live_theme() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
    while std::time::Instant::now() < deadline {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn names_become_safe_config_file_slugs() {
    assert_eq!(slugify("  Rosé / Pine!  "), "ros-pine");
    assert_eq!(slugify("Ocean  Blue"), "ocean-blue");
}

#[test]
fn custom_theme_ids_fall_back_and_skip_taken_ids() {
    let cases: [(&str, &[&str], &str); 5] = [
        ("Ocean Blue", &["ocean-blue"], "ocean-blue-2"),
        ("日本語", &[], "custom-theme"),
        ("!!!", &[], "custom-theme"),
        ("日本語", &["custom-theme"], "custom-theme-2"),
        (
            "日本語",
            &["custom-theme", "custom-theme-2"],
            "custom-theme-3",
        ),
    ];
    for (name, taken, expected) in cases {
        assert_eq!(
            custom_theme_id(name, |id| taken.contains(&id)),
            expected,
            "{name:?}"
        );
    }
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
dark_background = "#070b13"
lighter_background = "#142033"
foreground = "#a8dfff"
accent = "#00aaff"
selection = "#1d3350"
muted = "#3a5068"
color8 = "#99aabb"
"##,
        super::OmarchyVariant::Lighter,
    )
    .expect("valid Quattro colors should map");

    assert_eq!(theme.name, "Azure Glow");
    assert_eq!(theme.background, "#0a0f1a");
    assert_eq!(theme.surface, "#070b13");
    assert_eq!(theme.muted, "#142033");
    assert_eq!(theme.highlight, "#1d3350");
    assert_eq!(theme.border, "#3a5068");
    assert_eq!(theme.accent, "#00aaff");

    let minimal = tokens_from_quattro(
        "azure-glow",
        "background = \"#0a0f1a\"\nlighter_background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\ncolor8 = \"#99aabb\"\n",
        super::OmarchyVariant::Lighter,
    )
    .expect("required Quattro colors should map");
    assert_eq!(minimal.background, "#0a0f1a");
    assert_eq!(minimal.surface, "#0a0f1a");
    assert_ne!(minimal.muted, minimal.background);
    let derived = super::resolved_source_palette(&theme, None);
    for (source, expected) in [
        ("color2 = \"#00ff00\"", "#00ff00"),
        ("color2 = \"#00ff00\"\ngreen = \"#009900\"", "#009900"),
        ("green = \"invalid\"", derived.string.as_str()),
    ] {
        let source = format!(
            "background = \"#0a0f1a\"\nforeground = \"#a8dfff\"\naccent = \"#00aaff\"\nselection = \"#a8dfff\"\ncolor8 = \"#123247\"\n{source}"
        );
        let tokens = tokens_from_quattro("azure-glow", &source, super::OmarchyVariant::Lighter)
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
    let tokens = tokens_from_quattro("azure-glow", &source, super::OmarchyVariant::Lighter)
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
            super::OmarchyVariant::Lighter
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

#[test]
fn theme_and_omarchy_folders_are_watched_and_released_on_the_monitors_worker() {
    gtk_test(
        "ui::theme::tests::theme_and_omarchy_folders_are_watched_and_released_on_the_monitors_worker",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};

            crate::ui::preferences::fixtures::seed_omarchy_for_test();
            std::fs::create_dir_all(themes_directory()).expect("custom themes folder");
            let themes = FsQuirks::install(&themes_directory(), Quirks::default());
            let creating = crate::test_support::Gate::default();
            let omarchy = FsQuirks::install(
                &gtk::glib::home_dir(),
                Quirks {
                    before: vec![(FsOp::Watch, "*", 0, creating.racer())],
                    ..Quirks::default()
                },
            );
            let watched = super::OmarchyRefresh::read().directories.len();
            let pump_until = |what: &dyn Fn() -> String, done: &dyn Fn() -> bool| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !done() {
                    assert!(std::time::Instant::now() < deadline, "{}", what());
                    gtk::glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };
            let calls = || {
                [
                    themes.count(FsOp::Watch),
                    themes.count(FsOp::Unwatch),
                    omarchy.count(FsOp::Watch),
                    omarchy.count(FsOp::Unwatch),
                ]
            };
            let wait_for = |what: &str, expected: &dyn Fn([usize; 4]) -> bool| {
                pump_until(&|| format!("{what}: {:?}", calls()), &|| expected(calls()));
            };
            let still = |expected: [usize; 4], what: &str| {
                crate::storage::writer::settle();
                for _ in 0..50 {
                    gtk::glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                assert_eq!(calls(), expected, "{what}");
            };
            assert!(watched > 1, "the Omarchy theme folder and its ancestors");

            let manager = ThemeManager::shared();
            pump_until(&|| "the first watch starts".to_owned(), &|| {
                creating.reached()
            });
            crate::ui::preferences::fixtures::seed_omarchy_colors_for_test(
                "background = '#010203'\nforeground = '#ddeeff'\naccent = '#445566'\n",
            );
            let mut outside = azure_tokens();
            outside.name = "Outside".to_owned();
            std::fs::write(
                themes_directory().join("outside.toml"),
                toml::to_string(&outside).expect("theme file"),
            )
            .expect("a theme saved elsewhere");
            creating.release();
            wait_for("watched at start", &|calls| calls == [1, 0, watched, 0]);
            pump_until(
                &|| "changes made while the watches started are read".to_owned(),
                &|| {
                    crate::storage::writer::settle();
                    manager
                        .omarchy_state
                        .borrow()
                        .as_ref()
                        .is_some_and(|state| state.colors.contains("#010203"))
                        && manager
                            .themes()
                            .iter()
                            .any(|theme| theme.tokens.name == "Outside")
                },
            );

            let mut tokens = azure_tokens();
            tokens.name = "Watched".to_owned();
            save_custom_theme(&manager, tokens).expect("custom theme saves");
            manager.refresh_omarchy_state();
            still(
                [1, 0, watched, 0],
                "a save and a refresh keep the watches they have",
            );

            let theme_folder = super::omarchy_state_dir().join("theme");
            std::fs::remove_dir_all(&theme_folder).expect("Omarchy theme folder removed");
            manager.refresh_omarchy_state();
            pump_until(&|| "the removed folder is read".to_owned(), &|| {
                crate::storage::writer::settle();
                manager.omarchy_state.borrow().is_none()
            });
            crate::ui::preferences::fixtures::seed_omarchy_for_test();
            manager.refresh_omarchy_state();
            wait_for(
                "only the folder that came back is watched again",
                &|[themes_watched, themes_released, watching, released]| {
                    [themes_watched, themes_released, watching] == [1, 0, watched + 1]
                        && released <= 1
                },
            );
        },
    );
}

#[test]
fn non_ascii_theme_names_save_under_a_fallback_id() {
    gtk_test(
        "ui::theme::tests::non_ascii_theme_names_save_under_a_fallback_id",
        || {
            let directory = themes_directory();
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");

            let mut tokens = azure_tokens();
            tokens.name = "日本語".to_owned();
            let held = crate::storage::writer::hold();
            let saved = Rc::new(std::cell::RefCell::new(None));
            let done = saved.clone();
            manager.save_custom_theme(tokens, move |result| {
                done.replace(Some(result));
            });
            for _ in 0..20 {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert!(
                saved.borrow().is_none(),
                "the save waits for the settings writer"
            );
            assert!(!directory.join("custom-theme.toml").exists());
            assert_eq!(manager.selected_id(), "azure-glow");
            drop(held);
            crate::storage::writer::settle();
            let id = saved
                .take()
                .expect("theme save finished")
                .expect("a non-ASCII name saves");
            assert_eq!(id, "custom-theme");
            let saved = std::fs::read_to_string(directory.join("custom-theme.toml"))
                .expect("saved theme file");
            assert_eq!(
                toml::from_str::<ThemeTokens>(&saved)
                    .expect("saved theme parses")
                    .name,
                "日本語"
            );
            assert!(
                manager
                    .themes()
                    .iter()
                    .any(|theme| theme.id == "custom-theme"
                        && theme.custom
                        && theme.tokens.name == "日本語")
            );
            assert_eq!(manager.selected_id(), "custom-theme");

            let files_before = std::fs::read_dir(&directory).expect("themes").count();
            let mut blank = azure_tokens();
            blank.name = "   ".to_owned();
            let error = save_custom_theme(&manager, blank).expect_err("a blank name is rejected");
            assert_eq!(error.to_string(), "Enter a theme name");
            assert_eq!(
                std::fs::read_dir(&directory).expect("themes").count(),
                files_before
            );
        },
    );
}

#[test]
fn file_icon_colors_repaint_catppuccin_icons_live_across_images() {
    gtk_test(
        "ui::theme::tests::file_icon_colors_repaint_catppuccin_icons_live_across_images",
        || {
            use crate::ui::preferences::{FileIconColors, PreferenceManager};

            let manager = super::ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");
            let preferences = PreferenceManager::shared();
            preferences.set_file_icon_colors(FileIconColors::Brand);

            let name = "strata-ctp-rust";
            let images = [
                crate::assets::primary_icon(name, 16),
                crate::assets::primary_icon(name, 16),
            ];
            let brand = images[0].paintable().expect("rendered file icon");
            assert_eq!(images[1].paintable().as_ref(), Some(&brand));

            preferences.set_file_icon_colors(FileIconColors::Theme);
            let themed = images[0].paintable().expect("repainted file icon");
            assert_ne!(themed, brand);
            assert_eq!(images[1].paintable().as_ref(), Some(&themed));
            let later = crate::assets::primary_icon(name, 16);
            assert_eq!(
                later.paintable().as_ref(),
                Some(&themed),
                "icons created after the change render the same way"
            );

            preferences.set_file_icon_colors(FileIconColors::Brand);
            for image in images.iter().chain([&later]) {
                assert_eq!(image.paintable().as_ref(), Some(&brand));
            }
        },
    );
}

#[test]
fn desktop_gtk_theme_and_gtk_css_do_not_restyle_widgets() {
    let desktop = tempfile::tempdir().expect("desktop config");
    let config = desktop.path().join("config");
    let data = desktop.path().join("data");
    let gtk_config = config.join("gtk-4.0");
    let theme = data.join("themes/Scratch/gtk-4.0");
    std::fs::create_dir_all(&gtk_config).expect("gtk config");
    std::fs::create_dir_all(&theme).expect("theme directory");
    std::fs::write(
        gtk_config.join("settings.ini"),
        "[Settings]\ngtk-theme-name=Scratch\ngtk-application-prefer-dark-theme=0\n",
    )
    .expect("settings.ini");
    std::fs::write(
        theme.join("gtk.css"),
        "@import url(\"resource:///org/gtk/libgtk/theme/Default/gtk.css\");\nlabel { color: rgb(0, 255, 0); }\n",
    )
    .expect("desktop theme");
    std::fs::write(
        gtk_config.join("gtk.css"),
        "window, label { color: rgb(255, 0, 0); }\n",
    )
    .expect("user stylesheet");
    gtk_test_with_env(
        "ui::theme::tests::desktop_gtk_theme_and_gtk_css_do_not_restyle_widgets",
        [
            ("GTK_THEME", String::new()),
            ("XDG_CONFIG_HOME", config.display().to_string()),
            ("XDG_DATA_HOME", data.display().to_string()),
        ],
        || {
            let settings = gtk::Settings::default().expect("settings");
            assert_eq!(settings.gtk_theme_name().as_deref(), Some("Scratch"));

            let manager = ThemeManager::shared();
            let window = gtk::Window::new();
            let label = gtk::Label::new(Some("isolated"));
            window.set_child(Some(&label));
            window.present();
            for _ in 0..20 {
                gtk::glib::MainContext::default().iteration(false);
            }

            assert_eq!(settings.gtk_theme_name().as_deref(), Some(PINNED_GTK_THEME));
            for variant in ["gtk.css", "gtk-dark.css"] {
                let path = format!("/org/gtk/libgtk/theme/{PINNED_GTK_THEME}/{variant}");
                if let Err(error) =
                    gtk::gio::resources_get_info(&path, gtk::gio::ResourceLookupFlags::NONE)
                {
                    panic!("{path} should be bundled: {error}");
                }
            }
            let expected =
                gtk::gdk::RGBA::parse(&manager.starter_tokens().text).expect("text color");
            let actual = label.color();
            assert!(
                has_color(&label, &expected),
                "label color {actual} should be Strata's text color {expected}"
            );
            window.close();
        },
    );
}

#[test]
fn corner_roundness_scales_the_stylesheet_before_settings_open_and_live() {
    gtk_test(
        "ui::theme::tests::corner_roundness_scales_the_stylesheet_before_settings_open_and_live",
        || {
            let settings = crate::ui::preferences::config_directory().join("settings.toml");
            std::fs::create_dir_all(settings.parent().expect("config directory"))
                .expect("config directory");
            std::fs::write(&settings, "corner_roundness = 0\n").expect("seed settings");
            let preferences = PreferenceManager::shared();
            let manager = ThemeManager::shared();
            let parse_errors = Rc::new(Cell::new(0));
            let errors = parse_errors.clone();
            manager
                .base_provider
                .connect_parsing_error(move |_, _, _| errors.set(errors.get() + 1));

            // GTK serializes the parsed sheet as one longhand per corner.
            let radii = |css: &str| -> Vec<String> {
                css.lines()
                    .filter_map(|line| line.trim().split_once("-radius: "))
                    .map(|(_, value)| value.trim_end_matches(';').to_owned())
                    .collect()
            };
            let squared = radii(&manager.base_stylesheet());
            assert!(
                !squared.is_empty(),
                "stylesheet keeps its radius declarations"
            );
            assert!(
                squared
                    .iter()
                    .all(|value| value == "0px" || value == "0" || value.ends_with('%')),
                "saved roundness 0 squares every pixel radius: {squared:?}"
            );

            preferences.set_corner_roundness(CornerRoundness::new(200));
            let doubled = radii(&manager.base_stylesheet());
            assert!(doubled.contains(&"10px".to_owned()));
            assert!(!doubled.contains(&"5px".to_owned()));
            assert_eq!(parse_errors.get(), 0, "scaled stylesheet parses cleanly");
        },
    );
}

#[test]
fn interface_font_applies_before_settings_open_and_live_while_previews_stay_monospace() {
    gtk_test(
        "ui::theme::tests::interface_font_applies_before_settings_open_and_live_while_previews_stay_monospace",
        || {
            let settings = crate::ui::preferences::config_directory().join("settings.toml");
            std::fs::create_dir_all(settings.parent().expect("config directory"))
                .expect("config directory");
            std::fs::write(&settings, "interface_font = \"Fixture Sans\"\n")
                .expect("seed settings");
            let preferences = PreferenceManager::shared();
            let manager = ThemeManager::shared();
            let parse_errors = Rc::new(Cell::new(0));
            let errors = parse_errors.clone();
            manager
                .font_provider
                .connect_parsing_error(move |_, _, _| errors.set(errors.get() + 1));

            let interface = gtk::Label::new(Some("Interface"));
            let preview = gtk::Label::new(Some("Preview"));
            preview.add_css_class("preview-text");
            let menu = gtk::Label::new(Some("Menu item"));
            let popover = gtk::Popover::builder().child(&menu).build();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&interface);
            content.append(&preview);
            popover.set_parent(&content);
            let window = gtk::Window::builder().child(&content).build();
            window.present();
            let family = |label: &gtk::Label| {
                label.measure(gtk::Orientation::Horizontal, -1);
                label
                    .pango_context()
                    .font_description()
                    .and_then(|description| description.family())
                    .map(|family| family.to_string())
                    .unwrap_or_default()
            };
            let font_name = || {
                gtk::Settings::default()
                    .and_then(|settings| settings.gtk_font_name())
                    .map(|name| name.to_string())
                    .unwrap_or_default()
            };

            assert!(family(&interface).starts_with("Fixture Sans,JetBrains Mono"));
            assert!(
                family(&menu).starts_with("Fixture Sans"),
                "popovers follow the font"
            );
            assert!(font_name().starts_with("Fixture Sans,JetBrains Mono"));
            assert!(family(&preview).starts_with("JetBrains Mono"));

            preferences.set_interface_font(Some("Other \"Quoted\" Sans".into()));
            assert!(family(&interface).starts_with("Other \"Quoted\" Sans,JetBrains Mono"));
            assert!(family(&menu).starts_with("Other \"Quoted\" Sans"));

            preferences.set_interface_font(None);
            assert!(family(&interface).starts_with("JetBrains Mono"));
            assert!(font_name().starts_with("JetBrains Mono"));
            assert!(family(&preview).starts_with("JetBrains Mono"));
            assert_eq!(parse_errors.get(), 0, "font stylesheet parses cleanly");
            popover.unparent();
            window.destroy();
        },
    );
}

#[test]
fn user_stylesheet_overrides_strata_and_reloads_live() {
    gtk_test(
        "ui::theme::tests::user_stylesheet_overrides_strata_and_reloads_live",
        || {
            let config = crate::ui::preferences::config_directory();
            std::fs::create_dir_all(&config).expect("config directory");
            let stylesheet = config.join("style.css");

            let manager = ThemeManager::shared();
            std::fs::write(
                &stylesheet,
                "label { color: nope; }\nlabel { color: rgb(255, 0, 0); }\n",
            )
            .expect("user stylesheet");
            let window = gtk::Window::new();
            let label = gtk::Label::new(Some("styled"));
            window.set_child(Some(&label));
            window.present();
            let red = gtk::gdk::RGBA::parse("rgb(255, 0, 0)").expect("red");
            let green = gtk::gdk::RGBA::parse("rgb(0, 255, 0)").expect("green");
            let text = gtk::gdk::RGBA::parse(&manager.starter_tokens().text).expect("text color");
            wait_for_live_theme(|| has_color(&label, &red));

            std::fs::write(&stylesheet, "label { color: rgb(0, 255, 0); }\n")
                .expect("edited stylesheet");
            wait_for_live_theme(|| has_color(&label, &green));

            std::fs::remove_file(&stylesheet).expect("remove stylesheet");
            wait_for_live_theme(|| has_color(&label, &text));
            window.close();
        },
    );
}
