// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gio, prelude::*};

use crate::{
    app::Browser,
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::SearchItem,
    ui::{preview::PreviewDrawer, search::SearchDialog, theme::ThemeManager},
};

use super::WindowContent;

#[cfg(test)]
mod tests;

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    content: &WindowContent,
    preferences: &Rc<ThemeManager>,
) {
    let controller = content.browser.browser();
    let preview = content.preview.clone();
    let search_preferences = preferences.clone();
    let activate =
        Rc::new(move |item| activate_result(&controller, &preview, &search_preferences, item));
    let dismissed_root = content.blurred_root.clone();
    let dismissed_button = content.header.search.clone();
    let dismiss = Rc::new(move || {
        dismissed_root.set_blurred(false);
        dismissed_button.remove_css_class("active");
    });
    let dialog = SearchDialog::new(activate, dismiss);
    content.overlay.add_overlay(&dialog.widget());
    let toggle = toggle_handler(dialog, content, preferences);
    let clicked_search = toggle.clone();
    content
        .header
        .search
        .connect_clicked(move |_| clicked_search());
    let action = gio::SimpleAction::new("search", None);
    action.connect_activate(move |_, _| toggle());
    window.add_action(&action);
}

fn toggle_handler(
    dialog: SearchDialog,
    content: &WindowContent,
    preferences: &Rc<ThemeManager>,
) -> Rc<dyn Fn()> {
    let button = content.header.search.clone();
    let root = content.blurred_root.clone();
    let preferences = preferences.clone();
    Rc::new(move || {
        if dialog.is_visible() {
            dialog.hide();
            return;
        }
        let roots = super::super::devices::global_search_roots();
        button.add_css_class("active");
        root.set_blurred(true);
        dialog.show(roots, preferences.sort_preferences().show_hidden);
    })
}

fn activate_result(
    controller: &Rc<Browser>,
    preview: &PreviewDrawer,
    preferences: &ThemeManager,
    item: SearchItem,
) {
    let location = Location::local(item.path.clone());
    if item.is_directory {
        preview.close();
        controller.navigate(location);
        return;
    }
    if let Some(parent) = item.path.parent() {
        controller.navigate(Location::local(parent));
    }
    if preferences.search_open_files_directly() {
        controller.open_location(location);
    } else {
        preview.show(
            FileEntry {
                location,
                native_name: item.path.file_name().unwrap_or_default().to_os_string(),
                thumbnail_path: None,
                display_name: item.name,
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
            },
            controller.active_depth(),
        );
    }
}
