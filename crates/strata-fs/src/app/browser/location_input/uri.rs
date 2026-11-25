// SPDX-License-Identifier: MIT

use crate::{
    model::{GIO_URI_BUILD_FLAGS, GIO_URI_PARSE_FLAGS, Location, lexically_normalized},
    services::{LocationValidationError, validate_uri_credentials},
};

pub(super) fn location_from_uri(input: &str) -> Result<Location, LocationValidationError> {
    if let Some(root) = virtual_root(input) {
        return Ok(Location::uri(root));
    }
    let scheme_end = input.find("://").unwrap_or_default();
    let scheme = &input[..scheme_end];
    let normalized = scheme.to_ascii_lowercase();
    if normalized == "file" {
        return location_from_file_uri(input);
    }
    if !matches!(
        normalized.as_str(),
        "smb"
            | "sftp"
            | "ftp"
            | "ftps"
            | "dav"
            | "davs"
            | "trash"
            | "network"
            | "recent"
            | crate::model::ADMIN_SCHEME
    ) {
        return Err(LocationValidationError::UnsupportedScheme(
            scheme.to_owned(),
        ));
    }
    // Typed text must take GIO's percent-encoded form to match listed children.
    let uri = validate_uri_credentials(&format!("{normalized}{}", &input[scheme_end..]))?;
    Ok(Location::uri(without_query_or_fragment(uri)))
}

fn location_from_file_uri(uri: &str) -> Result<Location, LocationValidationError> {
    let (path, host) =
        glib::filename_from_uri(uri).map_err(|_| LocationValidationError::InvalidUri)?;
    if host.is_some_and(|host| !host.is_empty() && !host.eq_ignore_ascii_case("localhost")) {
        return Err(LocationValidationError::UnsupportedShorthand(
            "A file:// URI must name a file on this computer. Use smb:// or sftp:// for another computer.",
        ));
    }
    Ok(Location::local(lexically_normalized(&path)))
}

/// GVfs folds a URI's query and fragment into the mount spec, so a backend that ignores them,
/// as SFTP does, mounts a spec the location never matches and its listing fails as not
/// mounted. No accepted scheme addresses files with either, so typed addresses drop both.
fn without_query_or_fragment(uri: String) -> String {
    let Ok(parsed) = glib::Uri::parse(&uri, GIO_URI_PARSE_FLAGS) else {
        return uri;
    };
    if parsed.query().is_none() && parsed.fragment().is_none() {
        return uri;
    }
    glib::Uri::build_with_user(
        GIO_URI_BUILD_FLAGS,
        &parsed.scheme(),
        parsed.user().as_deref(),
        None,
        None,
        parsed.host().as_deref(),
        parsed.port(),
        &parsed.path(),
        None,
        None,
    )
    .to_str()
    .to_string()
}

/// GIO's spelling of the roots of schemes without a host, which it also reads from
/// `network:` and `network://`.
const VIRTUAL_ROOTS: [(&str, &str); 3] = [
    ("network", "network:///"),
    ("trash", "trash:///"),
    ("recent", "recent:///"),
];

pub(super) fn virtual_root(input: &str) -> Option<&'static str> {
    let (scheme, rest) = input.split_once(':')?;
    if !rest.trim_start_matches('/').is_empty() || rest.len() > 3 {
        return None;
    }
    VIRTUAL_ROOTS
        .iter()
        .find(|(name, _)| scheme.eq_ignore_ascii_case(name))
        .map(|(_, root)| *root)
}

pub(super) fn is_uri_like(input: &str) -> bool {
    let Some(scheme_end) = input.find("://") else {
        return false;
    };
    let scheme = &input[..scheme_end];
    scheme.starts_with(|character: char| character.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')
        })
}
