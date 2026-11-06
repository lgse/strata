// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use super::{has_color, scheme_color_values, wait_for_live_theme};
use crate::test_support::{gtk_test, gtk_test_with_env, texture_has_visible_pixels};
use crate::ui::preferences::{CornerRoundness, PreferenceManager};
use crate::ui::theme::{
    PINNED_GTK_THEME, ThemeManager, ThemeTokens, azure_tokens, source_style_scheme_xml,
};
use gtk::prelude::*;

#[test]
fn applied_icon_colors_are_canonical_and_render_visible_strokes() {
    gtk_test(
        "ui::theme::tests::live::applied_icon_colors_are_canonical_and_render_visible_strokes",
        || {
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

#[test]
fn source_style_scheme_xml_canonicalizes_rgb_tokens_for_gtksourceview() {
    gtk_test(
        "ui::theme::tests::live::source_style_scheme_xml_canonicalizes_rgb_tokens_for_gtksourceview",
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
fn appearance_changes_keep_an_active_preview_until_it_is_cancelled() {
    gtk_test(
        "ui::theme::tests::live::appearance_changes_keep_an_active_preview_until_it_is_cancelled",
        || {
            use crate::ui::preferences::{PreferenceManager, TextSize};

            let manager = ThemeManager::shared();
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
fn file_icon_colors_repaint_catppuccin_icons_live_across_images() {
    gtk_test(
        "ui::theme::tests::live::file_icon_colors_repaint_catppuccin_icons_live_across_images",
        || {
            use crate::ui::preferences::{FileIconColors, PreferenceManager};

            let manager = ThemeManager::shared();
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
        "ui::theme::tests::live::desktop_gtk_theme_and_gtk_css_do_not_restyle_widgets",
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
        "ui::theme::tests::live::corner_roundness_scales_the_stylesheet_before_settings_open_and_live",
        || {
            seed_settings("corner_roundness = 0\n");
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
        "ui::theme::tests::live::interface_font_applies_before_settings_open_and_live_while_previews_stay_monospace",
        || {
            seed_settings("interface_font = \"Fixture Sans\"\n");
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
            // Interface labels, popovers (which follow the font), the GTK font name, and previews.
            let expect_fonts = |expected: [&str; 4]| {
                let actual = [
                    font_family(&interface),
                    font_family(&menu),
                    gtk_font_name(),
                    font_family(&preview),
                ];
                let matches = actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.starts_with(expected));
                assert!(matches, "{actual:?} should start with {expected:?}");
            };
            expect_fonts([
                "Fixture Sans,JetBrains Mono",
                "Fixture Sans",
                "Fixture Sans,JetBrains Mono",
                "JetBrains Mono",
            ]);

            preferences.set_interface_font(Some("Other \"Quoted\" Sans".into()));
            expect_fonts([
                "Other \"Quoted\" Sans,JetBrains Mono",
                "Other \"Quoted\" Sans",
                "",
                "",
            ]);

            preferences.set_interface_font(None);
            expect_fonts(["JetBrains Mono", "", "JetBrains Mono", "JetBrains Mono"]);
            assert_eq!(parse_errors.get(), 0, "font stylesheet parses cleanly");
            popover.unparent();
            window.destroy();
        },
    );
}

#[test]
fn user_stylesheet_overrides_strata_and_reloads_live() {
    gtk_test(
        "ui::theme::tests::live::user_stylesheet_overrides_strata_and_reloads_live",
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

/// Saved preferences as they are before Settings opens.
fn seed_settings(contents: &str) {
    let settings = crate::ui::preferences::config_directory().join("settings.toml");
    std::fs::create_dir_all(settings.parent().expect("config directory"))
        .expect("config directory");
    std::fs::write(&settings, contents).expect("seed settings");
}

fn font_family(label: &gtk::Label) -> String {
    label.measure(gtk::Orientation::Horizontal, -1);
    label
        .pango_context()
        .font_description()
        .and_then(|description| description.family())
        .map(|family| family.to_string())
        .unwrap_or_default()
}

fn gtk_font_name() -> String {
    gtk::Settings::default()
        .and_then(|settings| settings.gtk_font_name())
        .map(|name| name.to_string())
        .unwrap_or_default()
}
