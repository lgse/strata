// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::services::sanitize_uri_credentials;
use gio::prelude::*;

pub(crate) fn gio_file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

/// Builds a `Location` for a `gio::File`, preferring a native path only when
/// the file is genuinely on a local filesystem. A mounted GVfs backend (SMB,
/// SFTP, ...) can still return a `.path()` via its FUSE mirror even though the
/// file isn't native; using that path would leak the mirror's opaque
/// `/run/user/$UID/gvfs/...` location instead of the clean URI (lgse/strata#5).
/// GIO URIs keep GLib's normalized percent-encoding (`trash:///caf%E9.txt` is not decoded), so
/// names that are not valid UTF-8 round-trip. Returns `None` only for a genuinely malformed URI,
/// such as an invalid `%` escape.
pub(crate) fn location_for_file(file: &gio::File) -> Option<Location> {
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
pub(crate) fn reveal_target_for_file(file: &gio::File) -> Option<(Location, Location)> {
    let parent = file.parent()?;
    let name = file.basename()?;
    Some((
        location_for_file(&parent)?,
        location_for_file(&parent.child(name))?,
    ))
}

/// Query these when reading a type with [`content_type_of`].
pub(crate) const CONTENT_TYPE_ATTRIBUTES: &str =
    "standard::content-type,standard::fast-content-type";

/// Reads a queried file's type as GLib's default handler lookup does. Backends such as
/// GVfs SMB omit `standard::content-type` when the name alone leaves the type uncertain
/// (`.md`, `.py`, `.json`) and set only `standard::fast-content-type`. Reading the
/// attributes generically, rather than through `FileInfo::content_type`, avoids the GLib
/// critical that getter logs for a missing value.
pub(crate) fn content_type_of(info: &gio::FileInfo) -> Option<glib::GString> {
    [
        gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
        gio::FILE_ATTRIBUTE_STANDARD_FAST_CONTENT_TYPE,
    ]
    .into_iter()
    .find_map(|attribute| info.attribute_string(attribute))
}

#[cfg(test)]
mod tests;
