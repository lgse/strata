// SPDX-License-Identifier: MIT

use std::{collections::HashSet, rc::Rc};

use super::wait_for_live_theme;
use crate::test_support::gtk_test_with_env;
use crate::ui::preferences::PreferenceManager;
use crate::ui::theme::{
    ThemeManager, ThemeTokens, builtins, color_to_hex, new_source_buffer, parse_rgb_channels,
    register_document_buffer,
    system_appearance::{
        GNOME_COLOR_SCHEME_KEY, PortalAvailability, SYSTEM_THEME_PAIRS, SystemAppearance,
        gnome_interface_settings, system_theme_id,
    },
};
use ashpd::desktop::settings::ColorScheme;
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
            let targets = ThemedTargets::new();

            assert!(manager.is_system_appearance_available());
            assert!(manager.follows_system_appearance());
            assert_eq!(manager.selected_id(), "onedark-dark");
            assert_eq!(manager.effective_theme_id(), "one-light");
            let day = manager.appearance_tokens();
            targets.assert_applied(&manager, &day);

            settings
                .set_string(GNOME_COLOR_SCHEME_KEY, "prefer-dark")
                .expect("set night appearance");
            wait_for_live_theme(|| manager.effective_theme_id() == "onedark-dark");
            let night = manager.appearance_tokens();
            assert_ne!(day, night);
            assert_eq!(manager.selected_id(), "onedark-dark");
            targets.assert_applied(&manager, &night);

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
            targets.window.close();
        },
    );
}

#[test]
fn portal_scheme_overrides_gnome_and_no_preference_keeps_gnome_live() {
    gtk_test_with_env(
        "ui::theme::tests::system_appearance::portal_scheme_overrides_gnome_and_no_preference_keeps_gnome_live",
        [("GSETTINGS_BACKEND", "memory")],
        || {
            let settings = gnome_interface_settings().expect("GNOME color scheme schema");
            settings
                .set_string(GNOME_COLOR_SCHEME_KEY, "prefer-dark")
                .expect("set GNOME night appearance");
            let preferences = PreferenceManager::shared();
            preferences.set_theme_selection("system", Some("onedark-dark"));
            let manager = ThemeManager::shared();
            let targets = ThemedTargets::new();

            manager.set_portal_scheme(ColorScheme::PreferLight);
            assert_eq!(manager.effective_theme_id(), "one-light");
            let day = manager.appearance_tokens();
            targets.assert_applied(&manager, &day);

            manager.set_portal_scheme(ColorScheme::PreferDark);
            assert_eq!(manager.effective_theme_id(), "onedark-dark");
            let night = manager.appearance_tokens();
            assert_ne!(day, night);
            targets.assert_applied(&manager, &night);

            manager.set_portal_scheme(ColorScheme::NoPreference);
            assert_eq!(manager.effective_theme_id(), "onedark-dark");
            settings
                .set_string(GNOME_COLOR_SCHEME_KEY, "prefer-light")
                .expect("set GNOME day appearance");
            wait_for_live_theme(|| manager.effective_theme_id() == "one-light");
            targets.assert_applied(&manager, &day);
            targets.window.close();
        },
    );
}

#[test]
fn unavailable_portal_uses_gnome_without_changing_system_mode() {
    gtk_test_with_env(
        "ui::theme::tests::system_appearance::unavailable_portal_uses_gnome_without_changing_system_mode",
        [("GSETTINGS_BACKEND", "memory")],
        || {
            let settings = gnome_interface_settings().expect("GNOME color scheme schema");
            settings
                .set_string(GNOME_COLOR_SCHEME_KEY, "prefer-light")
                .expect("set GNOME day appearance");
            let preferences = PreferenceManager::shared();
            preferences.set_theme_selection("system", Some("onedark-dark"));
            let manager = ThemeManager::shared();
            let targets = ThemedTargets::new();

            manager.set_portal_scheme(ColorScheme::PreferDark);
            assert_eq!(manager.effective_theme_id(), "onedark-dark");
            manager.set_portal_unavailable();
            assert_eq!(manager.effective_theme_id(), "one-light");
            assert_eq!(preferences.theme_mode(), "system");
            targets.assert_applied(&manager, &manager.appearance_tokens());
            targets.window.close();
        },
    );
}

#[test]
fn unavailable_sources_repair_system_mode() {
    gtk_test_with_env(
        "ui::theme::tests::system_appearance::unavailable_sources_repair_system_mode",
        [("GSETTINGS_BACKEND", "memory")],
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_theme_selection("system", Some("onedark-dark"));
            let manager = Rc::new(ThemeManager::new(
                preferences.clone(),
                builtins(),
                SystemAppearance::for_test(None, PortalAvailability::Probing, None),
                None,
            ));

            manager.set_portal_unavailable();
            assert_eq!(preferences.theme_mode(), "theme");
            assert!(!manager.is_system_appearance_available());
        },
    );
}

struct ThemedTargets {
    window: gtk::Window,
    document: gtk::TextBuffer,
    source: sourceview5::Buffer,
}

impl ThemedTargets {
    fn new() -> Self {
        let document = gtk::TextBuffer::new(None);
        document.create_tag(Some("document-accent"), &[]);
        register_document_buffer(&document);
        Self {
            window: gtk::Window::new(),
            document,
            source: new_source_buffer(),
        }
    }

    fn assert_applied(&self, manager: &ThemeManager, tokens: &ThemeTokens) {
        #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
        let accent = self
            .window
            .style_context()
            .lookup_color("strata_accent")
            .expect("applied accent");
        let expected = gtk::gdk::RGBA::parse(&tokens.accent).expect("valid accent");
        assert_eq!(accent, expected);
        assert_eq!(manager.current_tokens().as_ref(), Some(tokens));
        assert_eq!(
            crate::assets::primary_icon_color(),
            color_to_hex(&tokens.accent)
        );
        assert_eq!(manager.active_model_palette().accent, {
            let [red, green, blue] = parse_rgb_channels(&tokens.accent).expect("accent RGB");
            (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
        });
        assert_eq!(
            self.document
                .tag_table()
                .lookup("document-accent")
                .expect("document accent tag")
                .foreground_rgba(),
            Some(expected)
        );
        let scheme = self.source.style_scheme().expect("source scheme");
        assert_eq!(
            scheme
                .style("text")
                .expect("source text style")
                .background()
                .as_deref(),
            Some(tokens.surface.as_str())
        );
    }
}
