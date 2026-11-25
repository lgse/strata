// SPDX-License-Identifier: MIT

use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use crate::{
    model::{Location, lexically_normalized},
    services::LocationValidationError,
};

mod shell_word;
mod shorthand;
mod uri;

pub(super) use shorthand::unsupported_shorthand_message;
use uri::{is_uri_like, location_from_uri, virtual_root};

/// A relative path resolves against the native folder `base`.
pub(super) fn location_from_input(
    input: &str,
    base: Option<&Path>,
) -> Result<Location, LocationValidationError> {
    location_from_input_with_home(input, base, &glib::home_dir())
}

/// Reads pasted text as a terminal or Copy path would have written it: one layer of
/// quotes, shell escapes, and `file://` URIs name the same location as the plain path.
pub(super) fn location_from_input_with_home(
    input: &str,
    base: Option<&Path>,
    home: &Path,
) -> Result<Location, LocationValidationError> {
    if let Some(uri) = typed_location_uri(input) {
        return location_from_uri(uri);
    }
    let path = typed_path(input);
    if path.as_os_str().is_empty() {
        return Err(LocationValidationError::Empty);
    }
    if let Some(message) = path.to_str().and_then(unsupported_shorthand_message) {
        return Err(LocationValidationError::UnsupportedShorthand(message));
    }
    let resolved = resolved_local_path(path, base, home)?;
    Ok(Location::local(lexically_normalized(&resolved)))
}

fn resolved_local_path(
    path: PathBuf,
    base: Option<&Path>,
    home: &Path,
) -> Result<PathBuf, LocationValidationError> {
    let bytes = path.as_os_str().as_bytes();
    if bytes == b"~" {
        return Ok(home.to_path_buf());
    }
    if let Some(relative) = bytes.strip_prefix(b"~/") {
        let start = relative.iter().take_while(|&&byte| byte == b'/').count();
        return Ok(home.join(OsStr::from_bytes(&relative[start..])));
    }
    if bytes.starts_with(b"~") {
        return Err(LocationValidationError::UnsupportedShorthand(
            "Only ~ and ~/ paths are supported for the current user's home directory.",
        ));
    }
    Ok(match base.filter(|_| path.is_relative()) {
        Some(base) => base.join(&path),
        None => path,
    })
}

/// The URI that typed text names, without one layer of surrounding quotes. Shell
/// escapes inside a URI are left alone.
pub fn typed_location_uri(input: &str) -> Option<&str> {
    let input = input.trim();
    let unquoted = shell_word::outer_quotes_removed(input).map_or(input, str::trim);
    (is_uri_like(unquoted) || virtual_root(unquoted).is_some()).then_some(unquoted)
}

fn typed_path(input: &str) -> PathBuf {
    let input = input.trim();
    match shell_word::unquoted(input) {
        Some(word) => PathBuf::from(OsString::from_vec(word.trim_ascii().to_vec())),
        None => PathBuf::from(input),
    }
}
