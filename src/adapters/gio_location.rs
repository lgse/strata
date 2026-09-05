// SPDX-License-Identifier: GPL-3.0-or-later

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
/// Returns `None` when GIO provides a malformed URI.
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

#[cfg(test)]
mod tests;
