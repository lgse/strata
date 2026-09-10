// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::ui::theme::ThemeManager;

use super::{
    append_heading, bindings::bind_switch, page_content, scrollable_page, settings_option,
};

pub(super) fn keybindings_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();
    append_heading(&content, "KEYBINDING HINTS");
    let (row, toggle) = settings_option(
        "Show keybinding hints",
        "Show navigation hints and paste availability at the bottom of every view. F1 opens the full reference even when hints are hidden.",
        manager.show_keybinding_hints(),
    );
    bind_switch(
        &manager,
        &toggle,
        ThemeManager::show_keybinding_hints,
        ThemeManager::set_show_keybinding_hints,
    );
    content.append(&row);
    append_heading(&content, "NAVIGATION");
    for (label, keys) in [
        ("Move through items", "↑ / ↓ (← / → in Icons)"),
        ("Jump to top / bottom", "Ctrl + ↑ / Ctrl + ↓"),
        ("Open item", "Enter"),
        ("Go to parent", "Alt + ↑"),
        ("Back / forward", "Alt + ← / →"),
        ("Move between column panes", "← / → (Columns)"),
        ("Focus pane navigation header", "↑ at top"),
        ("Sidebar to top navigation bar", "↑ from sidebar top"),
        ("Return from header to files", "↓"),
        ("Edit location", "Ctrl + L"),
        ("Filter items", "Ctrl + F"),
        ("Toggle sidebar", "Ctrl + B"),
    ] {
        append_keybinding(&content, label, keys);
    }

    append_heading(&content, "VIEW");
    append_keybinding(&content, "Toggle hidden files", "Ctrl + H  or  Ctrl + .");

    append_heading(&content, "FILE OPERATIONS");
    for (label, keys) in [
        ("Create new folder", "Ctrl + Shift + N"),
        ("Cut", "Ctrl + X"),
        ("Copy", "Ctrl + C"),
        ("Paste", "Ctrl + V"),
        ("Rename", "F2 / Ctrl + R"),
    ] {
        append_keybinding(&content, label, keys);
    }

    append_heading(&content, "APPLICATION");
    for (label, keys) in [
        ("Search", "Ctrl + K"),
        ("Open terminal", "Ctrl + T"),
        ("Refresh", "F5"),
        ("Open settings", "Ctrl + ,"),
        ("Shortcut reference", "F1"),
    ] {
        append_keybinding(&content, label, keys);
    }

    scrollable_page(&content, Some("settings-keybindings-scroll"))
}

fn append_keybinding(content: &gtk::Box, label: &str, keys: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("keybinding-row");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let keys = gtk::Label::new(Some(keys));
    keys.add_css_class("keybinding-keys");
    row.append(&label);
    row.append(&keys);
    content.append(&row);
}
