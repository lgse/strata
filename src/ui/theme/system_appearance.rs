// SPDX-License-Identifier: MIT

//! Follows GNOME's day and night appearance by switching between paired themes.

use std::rc::Rc;

use gtk::{gio, prelude::*};

use super::ThemeManager;

pub(super) const GNOME_INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
pub(super) const GNOME_COLOR_SCHEME_KEY: &str = "color-scheme";

/// Explicit families whose bundled light and dark palettes are intended to match.
pub(super) const SYSTEM_THEME_PAIRS: &[(&str, &str)] = &[
    ("atelier-cave", "atelier-cave-light"),
    ("atelier-dune", "atelier-dune-light"),
    ("atelier-forest", "atelier-forest-light"),
    ("atelier-heath", "atelier-heath-light"),
    ("atelier-lakeside", "atelier-lakeside-light"),
    ("atelier-seaside", "atelier-seaside-light"),
    ("ayu-dark", "ayu-light"),
    ("catppuccin-mocha", "catppuccin-latte"),
    ("classic-dark", "classic-light"),
    ("everforest-dark-hard", "everforest-light-hard"),
    ("everforest-dark-soft", "everforest-light-medium"),
    ("flexoki-dark", "flexoki-light"),
    ("github-dark", "github"),
    ("github-dark-high-contrast", "github-light-high-contrast"),
    ("google-dark", "google-light"),
    ("grayscale-dark", "grayscale-light"),
    ("gruvbox-dark-hard", "gruvbox-light-hard"),
    (
        "gruvbox-material-dark-medium",
        "gruvbox-material-light-medium",
    ),
    ("horizon-dark", "horizon-light"),
    ("material-darker", "material-lighter"),
    ("neovim-dark", "neovim-light"),
    ("nord", "nord-light"),
    ("onedark-dark", "one-light"),
    ("oxocarbon-dark", "oxocarbon-light"),
    ("papercolor-dark", "papercolor-light"),
    ("rose-pine", "rose-pine-dawn"),
    ("solarized-dark", "solarized-light"),
    ("tomorrow-night", "tomorrow"),
];

impl ThemeManager {
    pub(super) fn monitor_system_appearance(self: &Rc<Self>) {
        let Some(settings) = &self.system_settings else {
            return;
        };
        let weak = Rc::downgrade(self);
        settings.connect_changed(Some(GNOME_COLOR_SCHEME_KEY), move |settings, _| {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            let dark = gnome_prefers_dark(settings);
            if manager.system_dark.replace(dark) != dark && manager.follows_system_appearance() {
                manager.apply_selected();
            }
        });
    }
}

pub(super) fn gnome_interface_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup(GNOME_INTERFACE_SCHEMA, true)?;
    if !schema.has_key(GNOME_COLOR_SCHEME_KEY) {
        return None;
    }
    Some(gio::Settings::new_full(
        &schema,
        None::<&gio::SettingsBackend>,
        None,
    ))
}

pub(super) fn gnome_prefers_dark(settings: &gio::Settings) -> bool {
    color_scheme_prefers_dark(&settings.string(GNOME_COLOR_SCHEME_KEY))
}

pub(super) fn color_scheme_prefers_dark(value: &str) -> bool {
    value == "prefer-dark"
}

pub(super) fn system_theme_id(id: &str, dark: bool) -> &str {
    SYSTEM_THEME_PAIRS
        .iter()
        .find_map(|&(dark_id, light_id)| {
            (id == dark_id || id == light_id).then_some(if dark { dark_id } else { light_id })
        })
        .unwrap_or(id)
}
