// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::prelude::*;

use super::preferences::PreferenceManager;

pub(crate) const MODE_DESCRIPTION: &str = "Opinionated keyboard-centric mode with Yazi-style navigation. Disables some features. Toggle with Ctrl-Shift-M.";
pub(crate) const UNUSED_SUBTITLE: &str = "Not used in 10xer mode.";
pub(crate) const TAG_TEXT: &str = "10X";
pub(crate) const TAG_NAME: &str = "10xer mode";

pub(crate) fn chrome_suppressed() -> bool {
    PreferenceManager::shared().tenxer_mode()
}

pub(crate) fn hide_while_enabled(widget: &impl IsA<gtk::Widget>) {
    widget.add_css_class("tenxer-suppressed-chrome");
    PreferenceManager::shared().bind_preference(
        widget,
        PreferenceManager::tenxer_mode,
        |widget, enabled| {
            widget.set_visible(!enabled);
            widget.set_sensitive(!enabled);
        },
    );
}

/// The sort map handler restores device-order sensitivity when shown again.
pub(crate) fn hide_sort_direction_while_enabled(widget: &impl IsA<gtk::Widget>) {
    widget.add_css_class("tenxer-suppressed-chrome");
    widget.add_css_class("tenxer-sort-direction");
    PreferenceManager::shared().bind_preference(
        widget,
        PreferenceManager::tenxer_mode,
        |widget, enabled| {
            if enabled {
                widget.set_sensitive(false);
                widget.set_visible(false);
            } else {
                widget.set_sensitive(true);
                widget.set_visible(true);
            }
        },
    );
}

pub(crate) fn hide_filter_while_enabled(button: &gtk::ToggleButton, revealer: &gtk::Revealer) {
    button.add_css_class("tenxer-suppressed-chrome");
    revealer.add_css_class("tenxer-filter-revealer");
    let revealer = revealer.clone();
    let button_for_restore = button.clone();
    let primed = Cell::new(false);
    PreferenceManager::shared().bind_preference(
        button,
        PreferenceManager::tenxer_mode,
        move |widget, enabled| {
            widget.set_visible(!enabled);
            widget.set_sensitive(!enabled);
            if !primed.replace(true) {
                return;
            }
            if enabled {
                revealer.set_reveal_child(false);
            } else {
                revealer.set_reveal_child(button_for_restore.is_active());
            }
        },
    );
}

pub(crate) fn is_toggle_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SUPER_MASK)
        && matches!(key, gtk::gdk::Key::m | gtk::gdk::Key::M)
}
