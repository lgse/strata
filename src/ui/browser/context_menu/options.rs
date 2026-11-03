// SPDX-License-Identifier: MIT

use super::{SUBMENU_ENTRIES_CLASS, SUBMENU_ROW_CLASS, presentation};
use crate::ui::preferences::PreferenceManager;
use crate::ui::shortcut_reference::{self, ContextHint};
use gtk::prelude::*;

pub(super) fn submenu_row(header: &gtk::Button, entries: [&gtk::Button; 2]) -> gtk::Box {
    let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrapper.add_css_class(SUBMENU_ROW_CLASS);
    wrapper.append(header);
    let entries_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    entries_box.add_css_class(SUBMENU_ENTRIES_CLASS);
    for entry in entries {
        entries_box.append(entry);
    }
    wrapper.append(&entries_box);
    wrapper
}

pub(super) fn item_context_option(icon: &str, label: &str, hint: ContextHint) -> gtk::Button {
    item_context_option_with_icon(crate::assets::primary_icon(icon, 15), label, hint)
}

pub(super) fn item_context_danger_option(
    icon: &str,
    label: &str,
    hint: ContextHint,
) -> gtk::Button {
    item_context_option_with_icon(crate::assets::danger_icon(icon, 15), label, hint)
}

/// A row of icon, title, and shortcut hint inside `button`, styled with `class`'s
/// `-icon` and `-shortcut` variants.
fn option_row(
    icon: &gtk::Image,
    label: &str,
    hint: ContextHint,
    (button, class): (&gtk::Button, &str),
) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    icon.add_css_class(&format!("{class}-icon"));
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(icon);
    row.append(&title);
    let shortcut = gtk::Label::new(None);
    shortcut.add_css_class(&format!("{class}-shortcut"));
    row.append(&shortcut);
    bind_context_hint(button, &shortcut, label, hint);
    (row, title)
}

fn item_context_option_with_icon(icon: gtk::Image, label: &str, hint: ContextHint) -> gtk::Button {
    let translated = crate::i18n::tr(label);
    let button = presentation::menu_item_button();
    button.add_css_class("item-context-option");
    let (row, _) = option_row(&icon, &translated, hint, (&button, "item-context"));
    button.set_child(Some(&row));
    button
}

fn context_menu_row(
    icon: gtk::Image,
    label: &str,
    hint: ContextHint,
    button: &gtk::Button,
) -> (gtk::Box, gtk::Image, gtk::Label) {
    let translated = crate::i18n::tr(label);
    let (row, title) = option_row(&icon, &translated, hint, (button, "folder-context"));
    (row, icon, title)
}

pub(in crate::ui) fn context_menu_option(
    icon: &str,
    label: &str,
    hint: ContextHint,
) -> gtk::Button {
    let button = presentation::menu_item_button();
    let (row, _, _) = context_menu_row(crate::assets::primary_icon(icon, 15), label, hint, &button);
    button.add_css_class("folder-context-option");
    button.set_child(Some(&row));
    button
}

pub(in crate::ui::browser) fn context_menu_danger_option(
    icon: &str,
    label: &str,
    hint: ContextHint,
) -> gtk::Button {
    let button = presentation::menu_item_button();
    let (row, _, _) = context_menu_row(crate::assets::danger_icon(icon, 15), label, hint, &button);
    button.add_css_class("folder-context-option");
    button.add_css_class("danger");
    button.set_child(Some(&row));
    button
}

pub(super) fn context_menu_toggle_option(
    icon: &str,
    label: &str,
    hint: ContextHint,
) -> (gtk::Button, gtk::Image, gtk::Label) {
    let button = presentation::menu_item_button();
    let (row, icon, title) =
        context_menu_row(crate::assets::primary_icon(icon, 15), label, hint, &button);
    button.add_css_class("folder-context-option");
    button.set_child(Some(&row));
    (button, icon, title)
}

/// The accelerator is set before the hint text, whose change rebuilds the
/// native menu row that reads both.
fn bind_context_hint(button: &gtk::Button, shortcut: &gtk::Label, label: &str, hint: ContextHint) {
    let button = button.downgrade();
    let label = label.to_owned();
    PreferenceManager::shared().bind_preference(
        shortcut,
        // The same command shows other keys in the other map or once rebound.
        move |preferences| {
            (
                shortcut_reference::context_hint_for(hint, preferences),
                preferences.shown_keys(),
            )
        },
        move |widget, (hinted, _)| {
            let shown = widget
                .downcast_ref::<gtk::Label>()
                .expect("context hint label");
            let text = hinted
                .map(|keys| crate::ui::keys::label(&keys))
                .unwrap_or_default();
            let button = button.upgrade();
            if let Some(option) = button
                .as_ref()
                .and_then(|button| button.downcast_ref::<presentation::MenuOption>())
            {
                option.set_accelerator(
                    hinted
                        .and_then(shortcut_reference::menu_accelerator)
                        .map(|binding| binding.accelerator())
                        .unwrap_or_default(),
                );
            }
            shown.set_text(&text);
            shown.set_visible(!text.is_empty());
            if let Some(button) = button {
                crate::ui::accessibility::describe_menu_item(&button, &label, &text);
            }
        },
    );
}
