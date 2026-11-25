// SPDX-License-Identifier: MIT

use gio::prelude::*;
use strata_core::model::Location;
use strata_fs::adapters::{gio_file_for_location, location_for_file};
use strata_platform::clipboard::Files;

pub(crate) fn is_trash_root(location: &Location) -> bool {
    location.uri_value() == Some("trash:///")
}

pub(crate) fn is_trash(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

pub(crate) fn is_trash_item(location: &Location) -> bool {
    is_trash(location) && location.parent().as_ref().is_some_and(is_trash_root)
}

/// GVfs can move/delete whole trashed items, but cannot modify their children.
pub(crate) fn can_remove(location: &Location) -> bool {
    !is_trash(location) || is_trash_item(location)
}

/// Location equality that also accepts GIO-level equivalence (`file://` URI vs native path).
pub(crate) fn locations_equal(left: &Location, right: &Location) -> bool {
    left == right || gio_file_for_location(left).equal(&gio_file_for_location(right))
}

pub(crate) fn files_for_locations(locations: &[Location], cut: bool) -> Option<(Files, String)> {
    if locations.is_empty() {
        return None;
    }
    let uris: Vec<String> = locations
        .iter()
        .map(|loc| gio_file_for_location(loc).uri().into())
        .collect();
    let text = locations
        .iter()
        .map(Location::display_path)
        .collect::<Vec<_>>()
        .join("\n");
    Some((Files::new(uris, cut), text))
}

pub(crate) fn locations_from_files(files: &Files) -> Vec<Location> {
    files
        .uris
        .iter()
        .filter_map(|uri| location_for_file(&gio::File::for_uri(uri)))
        .collect()
}

pub(crate) fn copy_path_text(location: &Location, is_directory: bool) -> String {
    let Some(path) = location.native_path() else {
        return location.display_path();
    };
    // Clipboard text is UTF-8; a file URI preserves otherwise unrepresentable bytes.
    let mut text = path.to_str().map_or_else(
        || gio_file_for_location(location).uri().to_string(),
        str::to_owned,
    );
    if is_directory && !text.ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    text
}
