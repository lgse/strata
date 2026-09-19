// SPDX-License-Identifier: MIT

use super::{
    append_heading, bindings::bind_switch, page_content, scrollable_page, settings_option,
};
use crate::ui::preferences::PreferenceManager;
use gtk::prelude::*;
use std::rc::Rc;

const DEFAULT_CATEGORIES: &[&str] = &["Navigation", "Selection", "Files", "View", "Application"];
const MINIMAL_CATEGORIES: &[&str] = &[
    "Minimal navigation",
    "Minimal selection",
    "Minimal files",
    "Minimal prompts",
    "Minimal places",
    crate::ui::minimal_mode::LABELED_TITLE,
];

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

const MINIMAL_SHORTCUTS: &[(&str, &str, &str, &str)] = &[
    ("Minimal navigation", "Parent / leave preview", "", "h / ←"),
    (
        "Minimal navigation",
        "Open directory / enter preview",
        "",
        "l / →",
    ),
    ("Minimal navigation", "Next / previous", "", "j / k"),
    ("Minimal navigation", "First / last", "", "g g / G"),
    (
        "Minimal navigation",
        "Half / full page",
        "",
        "Ctrl + U / D / B / F",
    ),
    ("Minimal navigation", "Back / forward", "", "H / L"),
    (
        "Minimal navigation",
        "Preview / scroll preview",
        "",
        "i / J / K",
    ),
    ("Minimal selection", "Toggle item", "", "Space"),
    ("Minimal selection", "Visual select / unset", "", "v / V"),
    (
        "Minimal selection",
        "Select all / invert",
        "",
        "Ctrl + A / R",
    ),
    ("Minimal files", "Yank / cut / paste", "", "y / x / p"),
    ("Minimal files", "Paste; Replace on conflicts", "", "P"),
    (
        "Minimal files",
        "Trash / permanent delete",
        "",
        "d / D / Delete",
    ),
    (
        "Minimal files",
        "Rename / create",
        "footer prompt",
        "r / a / F2",
    ),
    ("Minimal files", "Open / Open With", "", "o / O"),
    ("Minimal files", "Copy path / name", "", "c c / c n"),
    ("Minimal files", "Hidden files", "", ". / Ctrl + H"),
    (
        "Minimal files",
        "Sort by name / modified / size / type",
        "shift reverses",
        ", a / m / s / e",
    ),
    ("Minimal prompts", "Find next / previous", "", "/ / ?"),
    ("Minimal prompts", "Repeat find", "", "n / N"),
    ("Minimal prompts", "Filter / recursive search", "", "f / s"),
    ("Minimal prompts", "History fuzzy / recent", "", "z / Z"),
    (
        "Minimal places",
        "Home / Downloads / Config / Trash",
        "",
        "g h / d / c / t",
    ),
    ("Minimal places", "Network / Recent", "", "g n / r"),
    (
        "Minimal places",
        "Documents / Pictures / Videos",
        "",
        "g k / p / v",
    ),
    ("Minimal places", "Pinned places", "sidebar order", "g 1–9"),
    (
        "Minimal places",
        "Go to path",
        "footer prompt; Tab cycles folders",
        "g Space",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "Leave minimal mode",
        "",
        "q",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "Close window",
        "",
        "Q",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "Toggle minimal mode",
        "",
        "Ctrl + Shift + M",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "Copy / cut / paste",
        "GUI",
        "Ctrl + C / X / V",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "Refresh / location / search",
        "",
        "F5 / Ctrl + L / Ctrl + K",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "View mode",
        "Columns / Icons / List",
        "Ctrl + 1 / 2 / 3",
    ),
    (
        crate::ui::minimal_mode::LABELED_TITLE,
        "New folder / properties",
        "",
        "Ctrl + Shift + N / Alt + Enter",
    ),
];

pub(super) fn search_text() -> String {
    SHORTCUTS
        .iter()
        .chain(MINIMAL_SHORTCUTS.iter())
        .map(|(category, label, note, keys)| format!("{category} {label} {note} {keys}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn keybindings_page(manager: Rc<PreferenceManager>) -> gtk::Widget {
    let content = page_content();
    let hints = super::settings_group(&content, "SHORTCUTS BUTTON");
    let (row, toggle) = settings_option(
        "Show F1 Shortcuts button",
        "Show the shortcuts button in the bottom bar. Item counts and clipboard status remain visible when the button is hidden. F1 always opens the full reference.",
        manager.show_keybinding_hints(),
    );
    bind_switch(
        &manager,
        &toggle,
        PreferenceManager::show_keybinding_hints,
        PreferenceManager::set_show_keybinding_hints,
    );
    row.add_css_class("keybinding-hints");
    hints.append(&row);
    let reference = gtk::Box::new(gtk::Orientation::Vertical, 0);
    super::search::tag(&reference, "Shortcut reference");
    content.append(&reference);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    toolbar.add_css_class("settings-library-toolbar");
    append_heading(&toolbar, "SHORTCUT REFERENCE");
    let count = gtk::Label::new(None);
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
    for (minimal, shortcuts, categories) in [
        (false, SHORTCUTS, DEFAULT_CATEGORIES),
        (true, MINIMAL_SHORTCUTS, MINIMAL_CATEGORIES),
    ] {
        for &category in categories {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let title = gtk::Label::new(Some(category));
            title.set_xalign(0.0);
            title.set_wrap(true);
            title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            title.add_css_class("shortcut-category");
            section.append(&title);
            let group = super::settings_group(&section, "");
            let mut rows = Vec::new();
            for &(_, label, note, keys) in shortcuts
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
            groups.push((minimal, section, rows));
        }
    }
    let empty = gtk::Label::new(Some("No shortcuts match your search."));
    empty.add_css_class("settings-option-description");
    empty.set_visible(false);
    reference.append(&empty);
    let groups = Rc::new(groups);
    let refresh = {
        let groups = groups.clone();
        let count = count.clone();
        let empty = empty.clone();
        let search = search.clone();
        Rc::new(move |minimal: bool| {
            refresh_visible_shortcuts(&groups, minimal, search.text().as_str(), &count, &empty);
        })
    };
    let on_search = refresh.clone();
    let manager_for_search = manager.clone();
    search.connect_changed(move |search| {
        clear.set_visible(!search.text().is_empty());
        on_search(manager_for_search.minimal_mode());
    });
    manager.bind_preference(&count, PreferenceManager::minimal_mode, {
        let refresh = refresh.clone();
        move |_, minimal| refresh(minimal)
    });
    scrollable_page(&content, Some("settings-keybindings-scroll"))
}

type ShortcutGroup = (bool, gtk::Box, Vec<(gtk::Box, String)>);

fn refresh_visible_shortcuts(
    groups: &[ShortcutGroup],
    minimal: bool,
    query: &str,
    count: &gtk::Label,
    empty: &gtk::Label,
) {
    let query = query.trim().to_lowercase();
    let mut matches = 0;
    for (is_minimal, section, rows) in groups {
        if *is_minimal != minimal {
            section.set_visible(false);
            for (row, _) in rows {
                row.set_visible(false);
            }
            continue;
        }
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
