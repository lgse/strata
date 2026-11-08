// SPDX-License-Identifier: MIT

//! Follows GNOME's day and night appearance by switching between paired themes.

use std::{cell::Cell, rc::Rc};

use gtk::{gio, prelude::*};

use super::ThemeManager;

pub(super) const GNOME_INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
pub(super) const GNOME_COLOR_SCHEME_KEY: &str = "color-scheme";

/// `(dark, light)` bundled theme families; unlisted themes look the same in both modes.
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

pub(super) struct SystemAppearance {
    settings: Option<gio::Settings>,
    dark: Cell<bool>,
}

impl SystemAppearance {
    pub(super) fn read() -> Self {
        let settings = gnome_interface_settings();
        let dark = settings.as_ref().is_some_and(gnome_prefers_dark);
        Self {
            settings,
            dark: Cell::new(dark),
        }
    }

    pub(super) fn is_available(&self) -> bool {
        self.settings.is_some()
    }
}

impl ThemeManager {
    pub fn is_system_appearance_available(&self) -> bool {
        self.system_appearance.is_available()
    }

    pub fn follows_system_appearance(&self) -> bool {
        self.preferences.theme_mode() == "system"
    }

    pub fn effective_theme_id(&self) -> String {
        let selected = self.selected_id();
        if self.follows_system_appearance() {
            system_theme_id(&selected, self.system_appearance.dark.get()).to_owned()
        } else {
            selected
        }
    }

    pub fn set_follow_system_appearance(&self, enabled: bool) {
        if enabled && !self.is_system_appearance_available() {
            return;
        }
        self.preview.take();
        let changed = self.follows_system_appearance() != enabled;
        let mode = if enabled { "system" } else { "theme" };
        self.preferences.set_theme_selection(mode, None);
        if !changed {
            self.apply_selected();
        }
    }

    pub(super) fn monitor_system_appearance(self: &Rc<Self>) {
        let Some(settings) = &self.system_appearance.settings else {
            return;
        };
        let weak = Rc::downgrade(self);
        settings.connect_changed(Some(GNOME_COLOR_SCHEME_KEY), move |settings, _| {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            let dark = gnome_prefers_dark(settings);
            if manager.system_appearance.dark.replace(dark) != dark
                && manager.follows_system_appearance()
            {
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

fn gnome_prefers_dark(settings: &gio::Settings) -> bool {
    settings.string(GNOME_COLOR_SCHEME_KEY) == "prefer-dark"
}

pub(super) fn system_theme_id(id: &str, dark: bool) -> &str {
    SYSTEM_THEME_PAIRS
        .iter()
        .find_map(|&(dark_id, light_id)| {
            (id == dark_id || id == light_id).then_some(if dark { dark_id } else { light_id })
        })
        .unwrap_or(id)
}
