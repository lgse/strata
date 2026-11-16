// SPDX-License-Identifier: MIT

//! Theme library, Omarchy Quattro, system light/dark appearance, and live `Look` updates
//! (`src/ui/theme/` on `strata_kit::theme`).

use std::{
    fs,
    path::{Path, PathBuf},
};

use gio::prelude::*;
use rune::Tx;
use strata_core::{
    model::{LoadHandle, Location},
    storage,
};
use strata_desktop::settings::{
    FileIconColors, OmarchyVariant, Preferences, ThemeAvailability, settings_path,
};
use strata_fs::adapters::{Watched, watch};
use strata_kit::{
    fonts::FAMILIES,
    icons::{catppuccin, lucide},
    look::{Look, LookParams},
    theme::{Library, Theme, omarchy, parse_custom, switch},
};

use crate::{App, settings::Saved, slots::Later};

const GNOME_INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
const GNOME_COLOR_SCHEME_KEY: &str = "color-scheme";

/// `(dark, light)` bundled theme families paired under Follow system light/dark mode.
const SYSTEM_THEME_PAIRS: &[(&str, &str)] = &[
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

#[derive(Clone, PartialEq, Eq)]
struct OmarchyState {
    slug: String,
    colors: String,
}

/// Holds the loaded theme catalog, Omarchy state, and GNOME light/dark monitor for the window.
pub(crate) struct ThemeEngine {
    library: Library,
    omarchy: Option<OmarchyState>,
    gnome_settings: Option<gio::Settings>,
    system_dark: bool,
    _watches: Vec<LoadHandle>,
}

impl ThemeEngine {
    pub(crate) fn load(settings: &mut Saved, later: Later<App>) -> Self {
        let library = Library::new(read_custom_themes());
        let mut omarchy = read_omarchy();
        if let Some(state) = &mut omarchy {
            state.slug =
                settings.mutate_in_place(|prefs| prefs.migrate_active_omarchy_theme(&state.slug));
        }
        let gnome_settings = gnome_interface_settings();
        let system_dark = gnome_settings.as_ref().is_some_and(gnome_prefers_dark);
        let omarchy_available = omarchy
            .as_ref()
            .and_then(|state| omarchy_theme_of(state, OmarchyVariant::default()))
            .is_some();
        let available = ThemeAvailability {
            system: gnome_settings.is_some(),
            omarchy: omarchy_available,
            settings_file_exists: settings_path().exists(),
        };
        settings.normalize_themes(|id| library.get(id).is_some(), available);
        let watches = install_watches(&gnome_settings, later);
        Self {
            library,
            omarchy,
            gnome_settings,
            system_dark,
            _watches: watches,
        }
    }

    pub(crate) fn library(&self) -> &Library {
        &self.library
    }

    pub(crate) fn system_available(&self) -> bool {
        self.gnome_settings.is_some()
    }

    pub(crate) fn omarchy_available(&self) -> bool {
        self.omarchy
            .as_ref()
            .and_then(|state| omarchy_theme_of(state, OmarchyVariant::default()))
            .is_some()
    }

    pub(crate) fn active_omarchy_slug(&self) -> Option<&str> {
        self.omarchy.as_ref().map(|state| state.slug.as_str())
    }

    pub(crate) fn resolved_theme(&self, saved: &Preferences) -> Theme {
        if saved.mode == "omarchy"
            && let Some(state) = &self.omarchy
        {
            let variant = saved.omarchy_variant(Some(&state.slug));
            if let Some(theme) = omarchy_theme_of(state, variant) {
                return theme;
            }
        }
        let selected = if saved.mode == "system" && self.system_available() {
            paired_theme_id(&saved.theme, self.system_dark)
        } else {
            saved.theme.as_str()
        };
        self.library.resolve(Some(selected)).clone()
    }

    pub(crate) fn build_look(&self, saved: &Preferences, animations_allowed: bool) -> Look {
        let theme = self.resolved_theme(saved);
        let dark_bg = theme.dark_background();
        let text_size = u16::try_from(saved.text_size.root_font_px()).unwrap_or(u16::MAX);
        let roundness = u16::try_from(saved.corner_roundness.percent()).unwrap_or(u16::MAX);
        let params = LookParams {
            roundness,
            frame_density: crate::appearance::density::FRAME.saved(saved),
            content_density: crate::appearance::density::CONTENT.saved(saved),
            text_size,
            font: saved.interface_font.clone(),
            element_glow: saved.element_glow,
            animations: animations_allowed && !saved.reduce_motion,
            ..LookParams::default()
        };
        let mut look = theme.look().with_params(&params);
        look.icons = match saved.file_icon_colors {
            FileIconColors::Brand => catppuccin::for_background(dark_bg),
            FileIconColors::Theme => lucide::SET,
        };
        look
    }

    pub(crate) fn apply(&self, look: &mut Look, saved: &Preferences, animations_allowed: bool) {
        *look = self.build_look(saved, animations_allowed);
    }

    pub(crate) fn apply_tx(&self, look: &mut Look, next: Look, tx: &mut Tx) {
        if next.font != look.font {
            match &next.font {
                Some(families) => tx.set_font_families(families.ui.clone(), families.mono.clone()),
                None => tx.set_font_families(FAMILIES.0, FAMILIES.1),
            }
        }
        switch(look, next, tx);
    }

    pub(crate) fn reload_custom(&mut self) -> bool {
        let next = Library::new(read_custom_themes());
        if next.themes() == self.library.themes() {
            return false;
        }
        self.library = next;
        true
    }

    pub(crate) fn reload_omarchy(&mut self, settings: &mut Saved) -> bool {
        let mut next = read_omarchy();
        if let Some(state) = &mut next {
            state.slug =
                settings.mutate_in_place(|prefs| prefs.migrate_active_omarchy_theme(&state.slug));
        }
        if next == self.omarchy {
            return false;
        }
        self.omarchy = next;
        true
    }

    pub(crate) fn update_system_dark(&mut self, dark: bool) -> bool {
        let changed = self.system_dark != dark;
        self.system_dark = dark;
        changed
    }
}

pub(crate) fn themes_directory() -> PathBuf {
    storage::config_directory().join("themes")
}

fn omarchy_state_dir() -> PathBuf {
    crate::launch::home_directory().join(".local/state/omarchy/current")
}

fn read_custom_themes() -> Vec<Theme> {
    let Ok(entries) = fs::read_dir(themes_directory()) else {
        return Vec::new();
    };
    let mut custom = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(theme) = parse_custom(stem, &contents) {
            custom.push(theme);
        }
    }
    custom
}

fn read_omarchy() -> Option<OmarchyState> {
    let dir = omarchy_state_dir();
    let slug = fs::read_to_string(dir.join("theme.name")).ok()?;
    let colors = fs::read_to_string(dir.join("theme/colors.toml")).ok()?;
    Some(OmarchyState {
        slug: slug.trim().to_owned(),
        colors,
    })
}

fn omarchy_theme_of(state: &OmarchyState, variant: OmarchyVariant) -> Option<Theme> {
    let mapped = match variant {
        OmarchyVariant::Lighter => omarchy::Variant::Lighter,
        OmarchyVariant::Darker => omarchy::Variant::Darker,
        OmarchyVariant::Contrast => omarchy::Variant::Contrast,
    };
    omarchy::theme(&state.slug, &state.colors, mapped)
}

fn paired_theme_id(id: &str, dark: bool) -> &str {
    for &(dark_id, light_id) in SYSTEM_THEME_PAIRS {
        if id == dark_id || id == light_id {
            return if dark { dark_id } else { light_id };
        }
    }
    id
}

fn gnome_interface_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup(GNOME_INTERFACE_SCHEMA, true)?;
    schema
        .has_key(GNOME_COLOR_SCHEME_KEY)
        .then(|| gio::Settings::new(GNOME_INTERFACE_SCHEMA))
}

fn gnome_prefers_dark(settings: &gio::Settings) -> bool {
    settings.string(GNOME_COLOR_SCHEME_KEY) == "prefer-dark"
}

fn install_watches(gnome: &Option<gio::Settings>, later: Later<App>) -> Vec<LoadHandle> {
    if let Some(settings) = gnome {
        let queue = later.clone();
        settings.connect_changed(Some(GNOME_COLOR_SCHEME_KEY), move |settings, _| {
            let dark = gnome_prefers_dark(settings);
            queue.then(move |app: &mut App, tx: &mut Tx| {
                if app.window.themes_mut().update_system_dark(dark) {
                    crate::shell::follow_look_tx(app, tx);
                    return true;
                }
                false
            });
        });
    }
    let mut watches = Vec::new();
    let custom_later = later.clone();
    let custom_live = later.clone();
    watches.push(watch(
        &Location::local(themes_directory()),
        Watched::Folder,
        move |_, _| queue_custom_reload(&custom_later),
        move || queue_custom_reload(&custom_live),
    ));
    for dir in omarchy_watch_dirs() {
        let om_later = later.clone();
        let om_live = later.clone();
        watches.push(watch(
            &Location::local(dir),
            Watched::Folder,
            move |_, _| queue_omarchy_reload(&om_later),
            move || queue_omarchy_reload(&om_live),
        ));
    }
    watches
}

fn omarchy_watch_dirs() -> Vec<PathBuf> {
    let state = omarchy_state_dir();
    let home = crate::launch::home_directory();
    let theme_target = fs::canonicalize(state.join("theme")).ok();
    theme_target
        .into_iter()
        .chain(
            state
                .ancestors()
                .take_while(|path| path.starts_with(&home))
                .filter(|path| Path::is_dir(path))
                .take(1)
                .map(Path::to_path_buf),
        )
        .collect()
}

fn queue_custom_reload(later: &Later<App>) {
    later.then(|app: &mut App, tx: &mut Tx| {
        if app.window.themes_mut().reload_custom() {
            crate::shell::follow_look_tx(app, tx);
            return true;
        }
        false
    });
}

fn queue_omarchy_reload(later: &Later<App>) {
    later.then(|app: &mut App, tx: &mut Tx| {
        let changed = crate::shell::reload_omarchy(app);
        if changed {
            crate::shell::follow_look_tx(app, tx);
        }
        changed
    });
}
