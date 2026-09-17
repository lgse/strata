// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, prelude::*};

use crate::{
    app::{Browser, BrowserEvent},
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{NavigationHistory, SearchItem},
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
    let history = NavigationHistory::shared();
    install_history_recorder(&controller, &history);
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
    let toggle = toggle_handler(dialog.clone(), content, preferences);
    let clicked_search = toggle.clone();
    content
        .header
        .search
        .connect_clicked(move |_| clicked_search());
    let action = gio::SimpleAction::new("search", None);
    action.connect_activate(move |_, _| toggle());
    window.add_action(&action);

    let jump = folder_jump_handler(dialog, content, history);
    let action = gio::SimpleAction::new("jump-folder", None);
    action.connect_activate(move |_, _| jump());
    window.add_action(&action);
}

#[derive(Default)]
struct VisitRecorder {
    // Failed loads stay pending so a later successful retry records the visit.
    pending: Vec<Option<Location>>,
}

impl VisitRecorder {
    fn handle(&mut self, event: &BrowserEvent) -> Option<std::path::PathBuf> {
        match event {
            BrowserEvent::Reset => self.pending.clear(),
            BrowserEvent::ColumnsTruncated { len } => self.pending.truncate(*len),
            BrowserEvent::ColumnAdded { depth, location } => {
                if self.pending.len() <= *depth {
                    self.pending.resize(depth + 1, None);
                }
                self.pending[*depth] = Some(location.clone());
            }
            BrowserEvent::LoadFinished { depth, .. } => {
                return self
                    .pending
                    .get_mut(*depth)
                    .and_then(Option::take)
                    .and_then(|location| location.native_path().map(std::path::Path::to_path_buf));
            }
            _ => {}
        }
        None
    }
}

fn install_history_recorder(controller: &Rc<Browser>, history: &Rc<NavigationHistory>) {
    let recorder = Rc::new(RefCell::new(VisitRecorder::default()));
    let history = history.clone();
    controller.observe(move |event| {
        if let Some(path) = recorder.borrow_mut().handle(event) {
            history.record(&path);
        }
    });
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

fn folder_jump_handler(
    dialog: SearchDialog,
    content: &WindowContent,
    history: Rc<NavigationHistory>,
) -> Rc<dyn Fn()> {
    let button = content.header.search.clone();
    let root = content.blurred_root.clone();
    Rc::new(move || {
        if dialog.is_visible() {
            dialog.hide();
            return;
        }
        button.remove_css_class("active");
        root.set_blurred(true);
        dialog.show_history(history.clone());
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
        preview.clear_target();
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
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            },
            controller.active_depth(),
        );
    }
}
