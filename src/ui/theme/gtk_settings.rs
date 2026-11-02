// SPDX-License-Identifier: MIT

//! The GTK settings Strata pins so the desktop's theme, contrast and font
//! rendering cannot reshape its widgets.

use gtk::{glib, prelude::*};

use super::font;

/// GTK's automatic font rendering hints glyph outlines but not their metrics,
/// so GPU glyph atlas slots are sized from unhinted extents and lose the top
/// pixel row at some sizes (GNOME/gtk#8395). Manual rendering hints metrics
/// too and follows the desktop's hinting settings. Application values outrank
/// the settings portal, so this holds for the whole process.
pub(super) fn pin_manual_font_rendering() {
    if let Some(settings) = gtk::Settings::default() {
        set_enum_setting(&settings, "gtk-font-rendering", "manual");
    }
}

/// An empty theme bundled in Strata's resources. GTK's own theme provider sits
/// below the reset, so loading a real theme there only adds selector matching.
/// Desktop selections arrive through the settings portal or `settings.ini`,
/// which application values outrank for the whole process.
pub(super) const PINNED_GTK_THEME: &str = "StrataEmpty";

/// GTK's built-in theme, restated above the reset.
const BASE_THEME: &str = "Default";

/// Keeps GTK's own theme empty and its contrast regular whatever the desktop
/// configures, so only Strata's stylesheets shape its widgets. Values are set
/// even when they already match: only application-sourced values survive later
/// portal updates. `GTK_THEME` is cleared before GTK starts because it bypasses
/// the setting.
pub(super) fn pin_base_theme() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    settings.set_gtk_theme_name(Some(PINNED_GTK_THEME));
    set_enum_setting(&settings, "gtk-interface-contrast", "no-preference");
}

/// The built-in theme's light or dark variant follows the active Strata theme
/// rather than the desktop color scheme.
pub(super) fn apply_base_variant(dark: bool) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let scheme = if dark { "dark" } else { "light" };
    if settings
        .find_property("gtk-interface-color-scheme")
        .is_some()
    {
        set_enum_setting(&settings, "gtk-interface-color-scheme", scheme);
    } else {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

pub(super) fn base_theme_import(dark: bool) -> String {
    let variant = if dark { "gtk-dark.css" } else { "gtk.css" };
    format!("@import url(\"resource:///org/gtk/libgtk/theme/{BASE_THEME}/{variant}\");")
}

/// Skips properties this GTK lacks.
fn set_enum_setting(settings: &gtk::Settings, property: &str, nick: &str) {
    let Some(spec) = settings.find_property(property) else {
        return;
    };
    if let Some(value) =
        glib::EnumClass::with_type(spec.value_type()).and_then(|class| class.to_value_by_nick(nick))
    {
        settings.set_property(property, value);
    }
}

pub(super) fn apply_interface_font(family: Option<&str>, root_font_px: f64) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_font_name(Some(&font::font_name(family, root_font_px)));
    }
}
