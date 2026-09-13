// SPDX-License-Identifier: MIT

use super::{
    append_heading, bindings::bind_switch, page_content, scrollable_page, settings_option,
};
use crate::ui::theme::ThemeManager;
use gtk::prelude::*;
use std::rc::Rc;

const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
    (
        "Navigation",
        "Move through items",
        "← / → in Icons view",
        "↑ / ↓",
    ),
    (
        "Navigation",
        "Jump to top / bottom",
        "",
        "Ctrl + ↑ / Ctrl + ↓",
    ),
    ("Navigation", "Open item", "", "Enter"),
    ("Navigation", "Go to parent folder", "", "Alt + ↑"),
    ("Navigation", "Back / forward", "", "Alt + ← / Alt + →"),
    (
        "Navigation",
        "Move between column panes",
        "Columns view",
        "← / →",
    ),
    ("Navigation", "Focus pane header", "when at top", "↑"),
    ("Navigation", "Focus sidebar", "when at left edge", "←"),
    ("Selection", "Select all", "", "Ctrl + A"),
    ("Selection", "Extend selection", "", "Shift + ↑ / Shift + ↓"),
    ("Selection", "Toggle item in selection", "", "Ctrl + Space"),
    ("Selection", "Clear selection", "", "Esc"),
    ("Files", "Quick preview", "", "Space"),
    ("Files", "Cut / copy / paste", "", "Ctrl + X / C / V"),
    ("Files", "Duplicate", "", "Ctrl + D"),
    ("Files", "Rename", "", "F2 / Ctrl + R"),
    ("Files", "Create new folder", "", "Ctrl + Shift + N"),
    ("Files", "Move to Trash", "", "Delete"),
    ("Files", "Delete permanently", "", "Shift + Delete"),
    ("Files", "Undo file operation", "", "Ctrl + Z"),
    ("Files", "Item properties", "", "Alt + Enter"),
    ("View", "Toggle hidden files", "", "Ctrl + H / Ctrl + ."),
    (
        "View",
        "Switch view",
        "Columns / Icons / List",
        "Ctrl + 1 / 2 / 3",
    ),
    ("View", "Increase text size", "", "Ctrl + +"),
    ("View", "Decrease text size", "", "Ctrl + −"),
    ("View", "Reset text size", "", "Ctrl + 0"),
    ("View", "Toggle sidebar", "", "Ctrl + B"),
    ("Application", "Edit location", "", "Ctrl + L"),
    ("Application", "Filter items", "", "Ctrl + F"),
    ("Application", "Search", "", "Ctrl + K"),
    ("Application", "Open terminal", "", "Ctrl + T"),
    ("Application", "Refresh", "", "F5"),
    ("Application", "Open settings", "", "Ctrl + ,"),
    ("Application", "Shortcut reference", "", "F1"),
    ("Application", "Toggle arrow-key scope", "", "Ctrl + \\"),
];

pub(super) fn search_text() -> String {
    SHORTCUTS
        .iter()
        .map(|(category, label, note, keys)| format!("{category} {label} {note} {keys}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn keybindings_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();
    let hints = super::settings_group(&content, "HINTS");
    let (row, toggle) = settings_option(
        "Show keybinding hints",
        "Show navigation and paste hints at the bottom of every view. F1 always opens the full reference.",
        manager.show_keybinding_hints(),
    );
    bind_switch(
        &manager,
        &toggle,
        ThemeManager::show_keybinding_hints,
        ThemeManager::set_show_keybinding_hints,
    );
    row.add_css_class("keybinding-hints");
    hints.append(&row);
    let (row, toggle) = settings_option(
        "Keep arrows in file list",
        "Stop arrow keys from leaving the file list. Use Tab or the mouse to reach the toolbar and sidebar.",
        manager.arrow_navigation_scoped(),
    );
    bind_switch(
        &manager,
        &toggle,
        ThemeManager::arrow_navigation_scoped,
        ThemeManager::set_arrow_navigation_scoped,
    );
    hints.append(&row);
    let reference = gtk::Box::new(gtk::Orientation::Vertical, 0);
    super::search::tag(&reference, "Shortcut reference");
    content.append(&reference);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    toolbar.add_css_class("settings-library-toolbar");
    append_heading(&toolbar, "SHORTCUT REFERENCE");
    let count = gtk::Label::new(Some(&format!("{} bindings", SHORTCUTS.len())));
    count.add_css_class("settings-option-description");
    count.add_css_class("settings-control-label");
    count.set_ellipsize(gtk::pango::EllipsizeMode::End);
    count.set_hexpand(true);
    count.set_xalign(0.0);
    toolbar.append(&count);
    let (search_overlay, search, clear) = super::search_field("Search actions or keys");
    search.add_css_class("shortcut-search");
    crate::ui::accessibility::set_label(&search, "Search actions or keys");
    toolbar.append(&search_overlay);
    reference.append(&toolbar);
    let mut groups = Vec::new();
    for category in ["Navigation", "Selection", "Files", "View", "Application"] {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(Some(category));
        title.set_xalign(0.0);
        title.add_css_class("shortcut-category");
        section.append(&title);
        let group = super::settings_group(&section, "");
        let mut rows = Vec::new();
        for &(_, label, note, keys) in SHORTCUTS
            .iter()
            .filter(|(group, _, _, _)| *group == category)
        {
            let row = append_keybinding(&group, label, note, keys);
            rows.push((
                row,
                format!("{category} {label} {note} {keys}").to_lowercase(),
            ));
        }
        reference.append(&section);
        groups.push((section, rows));
    }
    let empty = gtk::Label::new(Some("No shortcuts match your search."));
    empty.add_css_class("settings-option-description");
    empty.set_visible(false);
    reference.append(&empty);
    search.connect_changed(move |search| {
        clear.set_visible(!search.text().is_empty());
        let query = search.text().trim().to_lowercase();
        let mut matches = 0;
        for (section, rows) in &groups {
            let mut visible = false;
            for (row, text) in rows {
                let matched = text.contains(&query);
                row.set_visible(matched);
                visible |= matched;
                matches += usize::from(matched);
            }
            section.set_visible(visible);
        }
        count.set_text(&format!("{matches} bindings"));
        empty.set_visible(matches == 0);
    });

    scrollable_page(&content, Some("settings-keybindings-scroll"))
}

fn append_keybinding(content: &gtk::Box, label: &str, note: &str, keys: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("keybinding-row");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    row.append(&label);
    let note = gtk::Label::new(Some(note));
    note.set_xalign(0.0);
    note.add_css_class("settings-option-description");
    note.add_css_class("settings-nowrap");
    note.set_visible(!note.text().is_empty());
    row.append(&note);
    let caps = keycaps(keys);
    caps.set_halign(gtk::Align::End);
    row.append(&caps);
    content.append(&row);
    row
}

pub(super) fn keycaps(keys: &str) -> super::wrap::WrapRow {
    let caps = super::wrap::WrapRow::new(6);
    caps.add_css_class("settings-keycaps");
    caps.set_hexpand(false);
    caps.set_valign(gtk::Align::Center);
    for key in keys.split_whitespace() {
        let label = gtk::Label::new(Some(key));
        label.set_halign(gtk::Align::End);
        label.add_css_class("settings-nowrap");
        label.add_css_class(if key == "+" || key == "/" {
            "keycap-separator"
        } else {
            "settings-keycap"
        });
        caps.append(&label);
    }
    caps
}
