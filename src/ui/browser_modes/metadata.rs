// SPDX-License-Identifier: MIT

//! Entry metadata a pane shows: its labels, the details still to fill, and the rows
//! a metadata fill updates in place.

use std::{collections::HashMap, path::Path};

use gtk::prelude::*;

use super::{list_pane::list_row_parts, pane::Pane, source_index::source_position_for_view};
use crate::{
    model::{FileEntry, MetadataValue},
    ui::{accessibility, browser, icons_cell, thumbnail},
};

pub(super) fn set_label_if_changed(label: &gtk::Label, text: &str) {
    if label.label() != text {
        label.set_label(text);
    }
}

pub(super) fn metadata_fill_position(
    position: Option<usize>,
    entry: &FileEntry,
    include_mode: bool,
    include_icon_details: bool,
) -> Option<usize> {
    position.filter(|_| {
        let mode_unknown = include_mode && entry.mode == MetadataValue::Unknown;
        let details_unknown = include_icon_details && icon_details_need_fill(entry);
        browser::metadata_needs_fill(entry)
            || mode_unknown
            || entry.is_directory()
            || details_unknown
    })
}

pub(in crate::ui) fn icon_details_need_fill(entry: &FileEntry) -> bool {
    if entry.is_directory() {
        return entry.child_count == MetadataValue::Unknown;
    }
    let path = Path::new(&entry.native_name);
    let dimensions_unknown =
        entry.image_dimensions == MetadataValue::Unknown && crate::services::is_image_path(path);
    let duration_unknown =
        entry.duration_seconds == MetadataValue::Unknown && crate::services::is_media_path(path);
    dimensions_unknown || duration_unknown
}

pub(in crate::ui) fn entry_size(entry: &FileEntry) -> String {
    if entry.is_directory() {
        return folder_item_info(entry).unwrap_or_default();
    }
    match entry.size {
        MetadataValue::Known(bytes) => browser::format_file_size(bytes),
        MetadataValue::Unknown | MetadataValue::Unavailable => String::new(),
    }
}

pub(super) fn entry_type(entry: &FileEntry) -> String {
    match crate::services::entry_type(entry) {
        crate::services::EntryType::Known(description) => description,
        kind => crate::i18n::tr(kind.description()),
    }
}

pub(super) fn entry_mode(entry: &FileEntry) -> String {
    match entry.mode {
        MetadataValue::Known(mode) => browser::format_permissions(mode),
        MetadataValue::Unknown | MetadataValue::Unavailable => String::new(),
    }
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

fn folder_item_info(entry: &FileEntry) -> Option<String> {
    match entry.child_count {
        MetadataValue::Known(0) => Some(crate::i18n::tr("No items")),
        MetadataValue::Known(count) => Some(crate::i18n::count_u64("items", count)),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

fn entry_icons_item_info(entry: &FileEntry) -> Option<String> {
    if entry.is_directory() {
        return folder_item_info(entry);
    }
    if let MetadataValue::Known(seconds) = entry.duration_seconds {
        return Some(format_duration(seconds));
    }
    if let MetadataValue::Known((width, height)) = entry.image_dimensions {
        return Some(format!("{width}×{height}"));
    }
    match entry.size {
        MetadataValue::Known(bytes) => Some(browser::format_file_size(bytes)),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

pub(super) fn set_icons_entry_details(card: &gtk::Box, entry: &FileEntry) {
    let Some(label) = icons_cell::details_label(card) else {
        return;
    };
    set_label_if_changed(&label, &entry_icons_item_info(entry).unwrap_or_default());
}

pub(super) fn update_bound_list_metadata(pane: &Pane, updates: &[(usize, FileEntry)]) {
    update_bound_metadata(pane, updates, update_list_row);
}

pub(super) fn update_bound_icons_metadata(pane: &Pane, updates: &[(usize, FileEntry)]) {
    update_bound_metadata(pane, updates, update_icons_card);
}

/// Applies filled metadata to every rendered item it belongs to, and forgets items
/// that were torn down.
fn update_bound_metadata(
    pane: &Pane,
    updates: &[(usize, FileEntry)],
    apply: fn(&gtk::ListItem, &gtk::Box, &FileEntry),
) {
    let updates: HashMap<usize, &FileEntry> = updates
        .iter()
        .map(|(position, entry)| (*position, entry))
        .collect();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let Some(entry) = source_position_for_view(
                &pane.source_index,
                Some(&section.view_model),
                item.position(),
            )
            .and_then(|position| updates.get(&position)) else {
                return true;
            };
            if let Ok(widget) = widget.downcast::<gtk::Box>() {
                apply(&item, &widget, entry);
            }
            true
        });
    }
}

fn update_list_row(item: &gtk::ListItem, row: &gtk::Box, entry: &FileEntry) {
    let Some((icon, name, _, mode, size, _, modified)) = list_row_parts(row) else {
        return;
    };
    if entry.is_locked_folder() {
        thumbnail::show_locked_folder(&icon);
        accessibility::describe_entry(item, &name.label(), Some(entry));
    }
    mode.set_label(&entry_mode(entry));
    size.set_label(&entry_size(entry));
    crate::ui::modified_date::set_modified_date(&modified, Some(entry), "—");
}

fn update_icons_card(item: &gtk::ListItem, card: &gtk::Box, entry: &FileEntry) {
    if entry.is_locked_folder() {
        if let Some((icon, _)) = icons_cell::parts(card) {
            thumbnail::show_locked_folder(&icon);
        }
        accessibility::describe_entry(item, &entry.display_name, Some(entry));
    }
    set_icons_entry_details(card, entry);
}
