// SPDX-License-Identifier: MIT

use super::{
    append_heading, bindings::bind_switch, page_content, scrollable_page, settings_option,
};
use crate::ui::preferences::PreferenceManager;
use gtk::prelude::*;
use std::rc::Rc;

use crate::ui::shortcut_reference;

pub(super) fn search_text() -> String {
    shortcut_reference::shortcuts(false)
        .chain(shortcut_reference::shortcuts(true))
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
    for minimal in [false, true] {
        for &category in shortcut_reference::categories(minimal) {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let title = gtk::Label::new(Some(category));
            title.set_xalign(0.0);
            title.set_wrap(true);
            title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            title.add_css_class("shortcut-category");
            section.append(&title);
            let group = super::settings_group(&section, "");
            let mut rows = Vec::new();
            for &(_, label, note, keys) in
                shortcut_reference::shortcuts(minimal).filter(|(group, _, _, _)| *group == category)
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
    let search_widget = search.downgrade();
    let empty_widget = empty.downgrade();
    let count_widget = count.downgrade();
    let refresh = Rc::new(move |count: &gtk::Widget, minimal: bool| {
        let (Some(search), Some(empty)) = (search_widget.upgrade(), empty_widget.upgrade()) else {
            return;
        };
        let Some(count) = count.downcast_ref::<gtk::Label>() else {
            return;
        };
        refresh_visible_shortcuts(&groups, minimal, search.text().as_str(), count, &empty);
    });
    let on_search = refresh.clone();
    let manager_for_search = manager.clone();
    search.connect_changed(move |search| {
        clear.set_visible(!search.text().is_empty());
        let Some(count) = count_widget.upgrade() else {
            return;
        };
        on_search(count.upcast_ref(), manager_for_search.minimal_mode());
    });
    manager.bind_preference(&count, PreferenceManager::minimal_mode, {
        let refresh = refresh.clone();
        move |anchor, minimal| refresh(anchor, minimal)
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
    note.set_wrap(true);
    note.set_wrap_mode(gtk::pango::WrapMode::WordChar);
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
