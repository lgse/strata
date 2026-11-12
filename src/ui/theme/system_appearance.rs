// SPDX-License-Identifier: MIT

//! Follows the desktop day and night preference through the portal, with GNOME
//! GSettings as a fallback.

use std::{cell::Cell, rc::Rc};

use ashpd::desktop::settings::ColorScheme;
#[cfg(not(test))]
use ashpd::desktop::settings::Settings;
#[cfg(not(test))]
use futures_lite::StreamExt as _;
#[cfg(not(test))]
use gtk::glib;
use gtk::{gio, prelude::*};
#[cfg(not(test))]
use std::time::Duration;

use super::ThemeManager;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PortalStartup {
    WaitForInitialValue,
    AsyncOnly,
}

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
    gnome_dark: Cell<bool>,
    portal: Cell<PortalAvailability>,
    portal_scheme: Cell<Option<ColorScheme>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PortalAvailability {
    Probing,
    Available,
    Unavailable,
}

#[cfg(not(test))]
const PORTAL_STARTUP_TIMEOUT: Duration = Duration::from_millis(500);

impl SystemAppearance {
    pub(super) fn read(startup: PortalStartup) -> Self {
        let settings = gnome_interface_settings();
        let gnome_dark = settings.as_ref().is_some_and(gnome_prefers_dark);
        let (portal, portal_scheme) = match startup {
            PortalStartup::WaitForInitialValue => match initial_portal_scheme() {
                Some(Ok(scheme)) => (PortalAvailability::Available, Some(scheme)),
                Some(Err(_)) | None => (PortalAvailability::Probing, None),
            },
            PortalStartup::AsyncOnly => (PortalAvailability::Probing, None),
        };
        Self {
            settings,
            gnome_dark: Cell::new(gnome_dark),
            portal: Cell::new(portal),
            portal_scheme: Cell::new(portal_scheme),
        }
    }

    pub(super) fn is_available(&self) -> bool {
        self.portal.get() != PortalAvailability::Unavailable || self.settings.is_some()
    }

    fn dark(&self) -> bool {
        match self.portal_scheme.get() {
            Some(ColorScheme::PreferDark) => true,
            Some(ColorScheme::PreferLight) => false,
            Some(ColorScheme::NoPreference) | None => self.gnome_dark.get(),
        }
    }
}

#[cfg(not(test))]
fn initial_portal_scheme() -> Option<Result<ColorScheme, ashpd::Error>> {
    async_io::block_on(futures_lite::future::or(
        async {
            Some(match Settings::new().await {
                Ok(settings) => settings.color_scheme().await,
                Err(error) => Err(error),
            })
        },
        async {
            async_io::Timer::after(PORTAL_STARTUP_TIMEOUT).await;
            None
        },
    ))
}

#[cfg(test)]
fn initial_portal_scheme() -> Option<Result<ColorScheme, ashpd::Error>> {
    None
}

#[cfg(test)]
impl SystemAppearance {
    pub(super) fn for_test(
        settings: Option<gio::Settings>,
        portal: PortalAvailability,
        scheme: Option<ColorScheme>,
    ) -> Self {
        let gnome_dark = settings.as_ref().is_some_and(gnome_prefers_dark);
        Self {
            settings,
            gnome_dark: Cell::new(gnome_dark),
            portal: Cell::new(portal),
            portal_scheme: Cell::new(scheme),
        }
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
            system_theme_id(&selected, self.system_appearance.dark()).to_owned()
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
        self.monitor_gnome_appearance();
        self.monitor_portal_appearance();
    }

    fn monitor_gnome_appearance(self: &Rc<Self>) {
        let Some(settings) = &self.system_appearance.settings else {
            return;
        };
        let weak = Rc::downgrade(self);
        settings.connect_changed(Some(GNOME_COLOR_SCHEME_KEY), move |settings, _| {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.update_gnome_appearance(gnome_prefers_dark(settings));
        });
    }

    fn monitor_portal_appearance(self: &Rc<Self>) {
        #[cfg(not(test))]
        self.follow_portal_appearance();
    }

    #[cfg(not(test))]
    fn follow_portal_appearance(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(settings) = Settings::new().await else {
                if let Some(manager) = weak.upgrade() {
                    manager.set_portal_unavailable();
                }
                return;
            };
            // Subscribed before the read so a change between the two is not lost.
            let changes = settings.receive_color_scheme_changed().await;
            // The proxy and the subscription succeed without a portal; only the read fails.
            let scheme = settings.color_scheme().await;
            let Some(manager) = weak.upgrade() else {
                return;
            };
            match scheme {
                Ok(scheme) => manager.set_portal_scheme(scheme),
                Err(_) => manager.set_portal_unavailable(),
            }
            drop(manager);
            let Ok(mut changes) = changes else {
                return;
            };
            while let Some(scheme) = changes.next().await {
                let Some(manager) = weak.upgrade() else {
                    return;
                };
                manager.set_portal_scheme(scheme);
            }
        });
    }

    fn update_gnome_appearance(&self, dark: bool) {
        let before = self.system_appearance.dark();
        self.system_appearance.gnome_dark.set(dark);
        self.reapply_system_appearance_if_changed(before);
    }

    pub(super) fn set_portal_scheme(&self, scheme: ColorScheme) {
        let before = self.system_appearance.dark();
        let availability_changed = self
            .system_appearance
            .portal
            .replace(PortalAvailability::Available)
            != PortalAvailability::Available;
        self.system_appearance.portal_scheme.set(Some(scheme));
        self.reapply_system_appearance_if_changed(before);
        if availability_changed {
            self.theme_listeners.notify(self);
        }
    }

    pub(super) fn set_portal_unavailable(&self) {
        let before = self.system_appearance.dark();
        let availability_changed = self
            .system_appearance
            .portal
            .replace(PortalAvailability::Unavailable)
            != PortalAvailability::Unavailable;
        self.system_appearance.portal_scheme.set(None);
        self.reapply_system_appearance_if_changed(before);
        if availability_changed {
            if !self.system_appearance.is_available() && self.follows_system_appearance() {
                self.preferences.set_theme_selection("theme", None);
            } else {
                self.theme_listeners.notify(self);
            }
        }
    }

    fn reapply_system_appearance_if_changed(&self, before: bool) {
        if before != self.system_appearance.dark() && self.follows_system_appearance() {
            self.apply_selected();
        }
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
