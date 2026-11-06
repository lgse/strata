// SPDX-License-Identifier: MIT

//! The Settings navigation: the pages in order and the button built for each.

use gtk::{glib, prelude::*};

use super::keyboard;
use crate::assets::icons;

pub(super) struct NavigationEntry {
    pub(super) page: &'static str,
    pub(super) label: &'static str,
    pub(super) subtitle: &'static str,
    pub(super) icon: &'static str,
    pub(super) accessible_label: Option<&'static str>,
}

pub(super) fn page_shown(page: &str) -> bool {
    page != "updates" || crate::build_info::updater_enabled()
}

/// Settings pages in navigation order. Search finds page titles here and routes
/// to the button built from each entry.
pub(super) static NAVIGATION: [NavigationEntry; 8] = [
    NavigationEntry {
        page: "general",
        label: "General",
        subtitle: "Browsing, search, files",
        icon: icons::SLIDERS,
        accessible_label: None,
    },
    NavigationEntry {
        page: "theme",
        label: "Appearance",
        subtitle: "Theme, text, motion",
        icon: icons::PALETTE,
        accessible_label: Some("Appearance settings"),
    },
    NavigationEntry {
        page: "actions",
        label: "Actions",
        subtitle: "Custom actions",
        icon: icons::PLAY,
        accessible_label: None,
    },
    NavigationEntry {
        page: keyboard::PAGE,
        label: "Keyboard",
        subtitle: "Shortcuts for each mode",
        icon: icons::KEYBOARD,
        accessible_label: Some("Keyboard settings"),
    },
    NavigationEntry {
        page: "integrations",
        label: "Integrations",
        subtitle: "Apps and services",
        icon: icons::BLOCKS,
        accessible_label: None,
    },
    NavigationEntry {
        page: "updates",
        label: "Updates",
        subtitle: "Channel, release notes",
        icon: icons::DOWNLOADS,
        accessible_label: None,
    },
    NavigationEntry {
        page: "diagnostics",
        label: "Diagnostics",
        subtitle: "Troubleshooting",
        icon: icons::STETHOSCOPE,
        accessible_label: None,
    },
    NavigationEntry {
        page: "about",
        label: "About",
        subtitle: "Version and links",
        icon: icons::INFO,
        accessible_label: None,
    },
];

pub(super) fn navigation_button(entry: &NavigationEntry) -> (gtk::Button, gtk::Label, gtk::Box) {
    let label = entry.label;
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon_image = crate::assets::primary_icon(entry.icon, 18);
    let text = gtk::Label::new(None);
    text.set_markup(&format!(
        "{}\n<span size=\"small\" weight=\"normal\">{}</span>",
        glib::markup_escape_text(&crate::i18n::tr(label)),
        glib::markup_escape_text(&crate::i18n::tr(entry.subtitle))
    ));
    text.set_xalign(0.0);
    text.add_css_class("settings-nav-copy");
    content.append(&icon_image);
    content.append(&text);
    let button = gtk::Button::builder().child(&content).build();
    button.set_widget_name(entry.page);
    button.set_has_frame(false);
    button.set_cursor_from_name(Some("pointer"));
    super::super::accessibility::set_label(
        &button,
        &crate::i18n::tr(entry.accessible_label.unwrap_or(label)),
    );
    (button, text, content)
}
