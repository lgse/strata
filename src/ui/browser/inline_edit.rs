// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FileEntry, Location};
use crate::services::validate_basename;
use crate::ui::browser::ViewState;
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use std::rc::Rc;

pub(super) struct ActiveRename {
    pub(super) entry: FileEntry,
    pub(super) field: gtk::Entry,
    pub(super) label: gtk::Label,
    pub(super) spacer: gtk::Box,
    pub(super) size: gtk::Label,
}

pub(super) struct ActiveNewEntry {
    pub(super) location: Location,
    pub(super) is_directory: bool,
    pub(super) row: gtk::Box,
    pub(super) field: gtk::Entry,
}

/// Whether a name currently typed into a field should be visually flagged as
/// an error. An empty name is left unstyled: it's the normal starting state
/// (opening, cancelling, or succeeding a prompt all clear the field) rather
/// than a mistake the user made, even though it still can't be submitted.
/// Kept separate from `update_basename_validation` so it can be unit tested
/// without constructing a real GTK widget.
fn basename_field_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        None
    } else {
        validate_basename(name).err()
    }
}

/// Validates a name field live as it changes, including the programmatic
/// clears that happen when a prompt opens, cancels, or succeeds.
pub(in crate::ui) fn update_basename_validation(field: &gtk::Entry) -> bool {
    let text = field.text();
    match basename_field_error(text.as_str()) {
        None => {
            field.remove_css_class("error");
            field.set_tooltip_text(None);
            !text.is_empty()
        }
        Some(message) => {
            field.add_css_class("error");
            field.set_tooltip_text(Some(message));
            false
        }
    }
}

pub(in crate::ui) fn rename_stem_end(name: &str) -> i32 {
    let end = name
        .rfind('.')
        .filter(|position| *position > 0)
        .unwrap_or(name.len());
    name[..end].chars().count().min(i32::MAX as usize) as i32
}

impl ViewState {
    pub(super) fn begin_new_entry(
        self: &Rc<Self>,
        depth: usize,
        location: Location,
        is_directory: bool,
    ) {
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            self.cancel_new_entry();
            self.mode_views
                .borrow()
                .begin_new_entry(depth, is_directory);
            return;
        }
        self.cancel_new_entry();
        self.cancel_rename();
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return;
        };
        let icon_name = if is_directory {
            crate::assets::icons::FOLDER
        } else {
            crate::assets::icons::DOCUMENTS
        };
        crate::assets::set_primary_icon(&column.new_entry_icon, icon_name);
        column.new_entry_entry.set_text("");
        column.new_entry_entry.remove_css_class("error");
        column.new_entry_entry.set_tooltip_text(None);
        column.new_entry_row.set_visible(true);
        self.active_new_entry.replace(Some(ActiveNewEntry {
            location,
            is_directory,
            row: column.new_entry_row.clone(),
            field: column.new_entry_entry.clone(),
        }));
        column.new_entry_entry.grab_focus();
    }

    pub(super) fn submit_new_entry(self: &Rc<Self>, field: &gtk::Entry) {
        if !self
            .active_new_entry
            .borrow()
            .as_ref()
            .is_some_and(|active| active.field == *field)
        {
            return;
        }
        let name = field.text().to_string();
        if !update_basename_validation(field) {
            field.grab_focus();
            return;
        }
        let Some(active) = self.active_new_entry.take() else {
            return;
        };
        active.row.set_visible(false);
        field.set_text("");
        if active.is_directory {
            self.browser.create_directory(active.location, name);
        } else {
            self.browser.create_file(active.location, name);
        }
    }

    pub(super) fn cancel_new_entry(&self) -> bool {
        let Some(active) = self.active_new_entry.take() else {
            return false;
        };
        active.field.set_text("");
        active.field.remove_css_class("error");
        active.field.set_tooltip_text(None);
        active.row.set_visible(false);
        true
    }

    pub(super) fn begin_rename(self: &Rc<Self>) -> bool {
        self.cancel_new_entry();
        self.sync_mode_selection();
        let Some((depth, source_position, entry)) = self.browser.rename_item() else {
            return false;
        };
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self
                .mode_views
                .borrow()
                .begin_rename(depth, source_position, &entry);
        }
        self.cancel_rename();
        let columns = self.columns.borrow();
        let Some(column) = columns.get(depth) else {
            return false;
        };
        let Some(filtered_position) = column.map.view_position(source_position) else {
            return false;
        };
        let row = column.bound_rows.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == filtered_position).then(|| bound.row.upgrade())?
        });
        let Some(row) = row else {
            return false;
        };
        let Some(icon) = row.first_child() else {
            return false;
        };
        let Some(middle) = icon.next_sibling().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(editor) = middle
            .child()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Box>()
        else {
            return false;
        };
        let Some(label) = editor.first_child().and_downcast::<gtk::Label>() else {
            return false;
        };
        let Some(field) = label.next_sibling().and_downcast::<gtk::Entry>() else {
            return false;
        };
        let Some(spacer) = field.next_sibling().and_downcast::<gtk::Box>() else {
            return false;
        };
        let Some(size) = middle.last_child().and_downcast::<gtk::Label>() else {
            return false;
        };
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        field.set_sensitive(true);
        field.set_text(&entry.display_name);
        label.set_visible(false);
        spacer.set_visible(false);
        size.set_visible(false);
        field.set_visible(true);
        field.grab_focus();
        field.select_region(0, rename_stem_end(&entry.display_name));
        self.active_rename.replace(Some(ActiveRename {
            entry,
            field,
            label,
            spacer,
            size,
        }));
        true
    }

    pub(super) fn cancel_rename(&self) -> bool {
        if self.mode_views.borrow().cancel_rename() {
            return true;
        }
        let Some(rename) = self.active_rename.take() else {
            return false;
        };
        rename.field.remove_css_class("error");
        rename.field.set_tooltip_text(None);
        rename.field.set_visible(false);
        rename.field.set_sensitive(true);
        rename.label.set_visible(true);
        rename.spacer.set_visible(true);
        rename.size.set_visible(!rename.size.label().is_empty());
        true
    }

    pub(super) fn submit_rename(self: &Rc<Self>, field: &gtk::Entry) {
        // `Browser::rename` rejects an invalid basename by emitting `RenameFailed` before it
        // returns, and that handler reads `active_rename` to flag the field, so the borrow
        // taken to read the entry must be released first.
        let entry = {
            let active = self.active_rename.borrow();
            let Some(rename) = active.as_ref().filter(|rename| rename.field == *field) else {
                return;
            };
            rename.entry.clone()
        };
        let new_name = field.text().to_string();
        if new_name == entry.display_name {
            self.cancel_rename();
            self.browser.focus_active();
            return;
        }
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        field.set_sensitive(false);
        self.browser.rename(entry, new_name);
    }
}

#[cfg(test)]
mod tests;
