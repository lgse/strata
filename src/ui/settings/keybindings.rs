// SPDX-License-Identifier: MIT

use super::{
    append_heading, bindings::bind_switch, page_content, scrollable_page, settings_option,
};
use crate::ui::preferences::PreferenceManager;
use crate::ui::shortcut_reference;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

type KeybindingGroups = RefCell<Vec<(gtk::Box, Vec<(gtk::Box, String)>)>>;

pub(super) fn search_text() -> String {
    shortcut_reference::active_settings_bindings()
        .iter()
        .map(|binding| {
            format!(
                "{} {} {} {}",
                binding.category, binding.action, binding.note, binding.keys
            )
        })
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
    let experimental = gtk::Label::new(None);
    experimental.add_css_class("settings-option-description");
    experimental.add_css_class("tenxer-experimental");
    experimental.set_xalign(0.0);
    experimental.set_wrap(true);
    experimental.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    experimental.set_visible(false);
    reference.append(&experimental);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    toolbar.add_css_class("settings-library-toolbar");
    append_heading(&toolbar, "SHORTCUT REFERENCE");
    let count = gtk::Label::new(Some("0 bindings"));
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
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    reference.append(&list);
    let empty = gtk::Label::new(Some("No shortcuts match your search."));
    empty.add_css_class("settings-option-description");
    empty.set_visible(false);
    reference.append(&empty);
    let groups = Rc::new(KeybindingGroups::new(Vec::new()));
    let apply_filter: Rc<dyn Fn(&str)> = Rc::new({
        let groups = groups.clone();
        let count = count.clone();
        let empty = empty.clone();
        let clear = clear.clone();
        move |query: &str| {
            clear.set_visible(!query.is_empty());
            let query = query.trim().to_lowercase();
            let mut matches = 0;
            for (section, rows) in groups.borrow().iter() {
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
    });
    let filter_on_type = apply_filter.clone();
    search.connect_changed(move |search| {
        filter_on_type(&search.text());
    });
    let experimental_for_mode = experimental.clone();
    let list_for_mode = list.clone();
    let groups_for_mode = groups.clone();
    let filter_on_mode = apply_filter.clone();
    let search_for_mode = search.clone();
    manager.bind_preference(
        &reference,
        PreferenceManager::tenxer_mode,
        move |_, enabled| {
            experimental_for_mode.set_text(if enabled {
                shortcut_reference::EXPERIMENTAL_LABEL
            } else {
                ""
            });
            experimental_for_mode.set_visible(enabled);
            rebuild_bindings(&list_for_mode, &groups_for_mode, enabled);
            filter_on_mode(&search_for_mode.text());
        },
    );
    scrollable_page(&content, Some("settings-keybindings-scroll"))
}

fn rebuild_bindings(list: &gtk::Box, groups: &KeybindingGroups, tenxer: bool) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let bindings = shortcut_reference::settings_bindings(tenxer);
    let mut built = Vec::new();
    let mut index = 0;
    while index < bindings.len() {
        let category = bindings[index].category;
        let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(Some(category));
        title.set_xalign(0.0);
        title.add_css_class("shortcut-category");
        section.append(&title);
        let group = super::settings_group(&section, "");
        let mut rows = Vec::new();
        while index < bindings.len() && bindings[index].category == category {
            let binding = &bindings[index];
            let row = append_keybinding(&group, binding.action, binding.note, binding.keys);
            let text = format!(
                "{} {} {} {}",
                binding.category, binding.action, binding.note, binding.keys
            )
            .to_lowercase();
            rows.push((row, text));
            index += 1;
        }
        list.append(&section);
        built.push((section, rows));
    }
    *groups.borrow_mut() = built;
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
