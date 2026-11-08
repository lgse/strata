// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use super::wait_for_live_theme;
use crate::test_support::gtk_test_with_env;
use crate::ui::preferences::PreferenceManager;
use crate::ui::theme::{
    ThemeManager, ThemeTokens, builtins, color_to_hex, new_source_buffer, parse_rgb_channels,
    register_document_buffer,
    system_appearance::{
        GNOME_COLOR_SCHEME_KEY, SYSTEM_THEME_PAIRS, gnome_interface_settings, system_theme_id,
    },
};
use gtk::prelude::*;
use sourceview5::prelude::BufferExt as _;

#[test]
fn curated_system_theme_pairs_resolve_in_both_directions() {
    let ids: HashSet<_> = builtins().into_iter().map(|theme| theme.id).collect();
    for &(dark, light) in SYSTEM_THEME_PAIRS {
        assert!(ids.contains(dark), "missing dark theme {dark}");
        assert!(ids.contains(light), "missing light theme {light}");
        assert_eq!(system_theme_id(dark, true), dark);
        assert_eq!(system_theme_id(light, true), dark);
        assert_eq!(system_theme_id(dark, false), light);
        assert_eq!(system_theme_id(light, false), light);
    }
    assert_eq!(system_theme_id("tokyo-night", true), "tokyo-night");
    assert_eq!(system_theme_id("tokyo-night", false), "tokyo-night");
    assert_eq!(system_theme_id("custom-palette", true), "custom-palette");
    assert_eq!(system_theme_id("custom-palette", false), "custom-palette");
}

#[test]
fn system_appearance_applies_at_startup_and_follows_gnome_live() {
    gtk_test_with_env(
        "ui::theme::tests::system_appearance::system_appearance_applies_at_startup_and_follows_gnome_live",
        [("GSETTINGS_BACKEND", "memory")],
        || {
            let settings = gnome_interface_settings().expect("GNOME color scheme schema");
            settings
                .set_string(GNOME_COLOR_SCHEME_KEY, "default")
                .expect("set day appearance");
            let preferences = PreferenceManager::shared();
            preferences.set_theme_selection("system", Some("onedark-dark"));
            let manager = ThemeManager::shared();
            let window = gtk::Window::new();
            let document = gtk::TextBuffer::new(None);
            document.create_tag(Some("document-accent"), &[]);
            register_document_buffer(&document);
            let source = new_source_buffer();

            assert!(manager.is_system_appearance_available());
            assert!(manager.follows_system_appearance());
            assert_eq!(manager.selected_id(), "onedark-dark");
            assert_eq!(manager.effective_theme_id(), "one-light");
            let day = manager.appearance_tokens();
            assert_applied_system_tokens(&manager, &window, &document, &source, &day);

            settings
                .set_string(GNOME_COLOR_SCHEME_KEY, "prefer-dark")
                .expect("set night appearance");
            wait_for_live_theme(|| manager.effective_theme_id() == "onedark-dark");
            let night = manager.appearance_tokens();
            assert_ne!(day, night);
            assert_eq!(manager.selected_id(), "onedark-dark");
            assert_applied_system_tokens(&manager, &window, &document, &source, &night);

            for value in ["prefer-light", "default"] {
                settings
                    .set_string(GNOME_COLOR_SCHEME_KEY, value)
                    .expect("set day appearance");
                wait_for_live_theme(|| manager.effective_theme_id() == "one-light");
                assert_eq!(manager.appearance_tokens(), day, "{value}");
            }
            crate::storage::writer::settle();
            assert_eq!(preferences.theme_mode(), "system");
            assert_eq!(preferences.selected_theme_id(), "onedark-dark");
            window.close();
        },
    );
}

fn assert_applied_system_tokens(
    manager: &ThemeManager,
    window: &gtk::Window,
    document: &gtk::TextBuffer,
    source: &sourceview5::Buffer,
    tokens: &ThemeTokens,
) {
    #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
    let accent = window
        .style_context()
        .lookup_color("strata_accent")
        .expect("applied accent");
    let expected = gtk::gdk::RGBA::parse(&tokens.accent).expect("valid accent");
    assert_eq!(accent, expected);
    assert_eq!(
        crate::assets::primary_icon_color(),
        color_to_hex(&tokens.accent)
    );
    assert_eq!(manager.active_model_palette().accent, {
        let [red, green, blue] = parse_rgb_channels(&tokens.accent).expect("accent RGB");
        (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
    });
    assert_eq!(
        document
            .tag_table()
            .lookup("document-accent")
            .expect("document accent tag")
            .foreground_rgba(),
        Some(expected)
    );
    let scheme = source.style_scheme().expect("source scheme");
    assert_eq!(
        scheme
            .style("text")
            .expect("source text style")
            .background()
            .as_deref(),
        Some(tokens.surface.as_str())
    );
}
