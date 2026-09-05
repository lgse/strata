// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FileEntry, Location};
use crate::ui::browser::PinStatus;
use gtk::glib;
use std::path::Path;

pub(super) fn can_pin_entry(entry: &FileEntry, status: PinStatus) -> bool {
    entry.is_directory() && !is_trash_location(&entry.location) && status == PinStatus::Available
}

pub(in crate::ui) fn is_trash_root(location: &Location) -> bool {
    location.uri_value() == Some("trash:///")
}

pub(super) fn is_trash_location(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

pub(super) fn compact_display_path(location: &Location) -> String {
    location
        .native_path()
        .map(compact_native_path)
        .unwrap_or_else(|| location.display_path())
}

pub(super) fn compact_native_path(path: &Path) -> String {
    let home = glib::home_dir();
    if path == home {
        return "~".to_owned();
    }
    path.strip_prefix(&home)
        .ok()
        .map(|suffix| format!("~/{}", suffix.to_string_lossy()))
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests;
