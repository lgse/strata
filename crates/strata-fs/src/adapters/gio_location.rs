// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::services::sanitize_uri_credentials;
use gio::prelude::*;

pub fn gio_file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

pub fn location_for_file(file: &gio::File) -> Option<Location> {
    if file.is_native()
        && let Some(path) = file.path()
    {
        return Some(Location::local(path));
    }
    let uri = file.uri();
    let (sanitized, _) = sanitize_uri_credentials(&uri).ok()?;
    Some(Location::uri(sanitized))
}

/// Rebuild the child as GIO listings do, preserving location equality across URI spellings.
pub fn reveal_target_for_file(file: &gio::File) -> Option<(Location, Location)> {
    let parent = file.parent()?;
    let name = file.basename()?;
    Some((
        location_for_file(&parent)?,
        location_for_file(&parent.child(name))?,
    ))
}

pub const CONTENT_TYPE_ATTRIBUTES: &str = "standard::content-type,standard::fast-content-type";

pub fn content_type_of(info: &gio::FileInfo) -> Option<glib::GString> {
    [
        gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
        gio::FILE_ATTRIBUTE_STANDARD_FAST_CONTENT_TYPE,
    ]
    .into_iter()
    .find_map(|attribute| info.attribute_string(attribute))
}

#[cfg(test)]
mod tests;
