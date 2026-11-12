// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use crate::{
    model::{GIO_URI_BUILD_FLAGS, GIO_URI_PARSE_FLAGS, Location, lexically_normalized},
    services::{LocationValidationError, validate_uri_credentials},
};

pub(super) fn location_from_input(input: &str) -> Result<Location, LocationValidationError> {
    location_from_input_with_home(input, &glib::home_dir())
}

pub(super) fn location_from_input_with_home(
    input: &str,
    home: &Path,
) -> Result<Location, LocationValidationError> {
    if input == "~" {
        return Ok(Location::local(lexically_normalized(home)));
    }
    if let Some(relative) = input.strip_prefix("~/") {
        return Ok(Location::local(lexically_normalized(
            &home.join(relative.trim_matches('/')),
        )));
    }
    if input.starts_with('~') {
        return Err(LocationValidationError::UnsupportedShorthand(
            "Only ~ and ~/ paths are supported for the current user's home directory.",
        ));
    }
    if let Some(root) = virtual_root(input) {
        return Ok(Location::uri(root));
    }
    if !is_uri_like(input) {
        return Ok(Location::local(lexically_normalized(Path::new(input))));
    }
    let scheme_end = input.find("://").unwrap_or_default();
    let scheme = &input[..scheme_end];
    let normalized = scheme.to_ascii_lowercase();
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
            | "computer"
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
const VIRTUAL_ROOTS: [(&str, &str); 4] = [
    ("network", "network:///"),
    ("trash", "trash:///"),
    ("recent", "recent:///"),
    ("computer", "computer:///"),
];

fn virtual_root(input: &str) -> Option<&'static str> {
    let (scheme, rest) = input.split_once(':')?;
    if !rest.trim_start_matches('/').is_empty() || rest.len() > 3 {
        return None;
    }
    VIRTUAL_ROOTS
        .iter()
        .find(|(name, _)| scheme.eq_ignore_ascii_case(name))
        .map(|(_, root)| *root)
}

/// UNC paths (`\\host\share`, bare `//host/share`) and SCP-style addresses
/// (`user@host:path`) are deliberately not accepted as location-bar shorthand
/// (see lgse/strata#20) so a proper URI (`smb://`, `sftp://`, ...) is always
/// preserved verbatim rather than being guessed at. Report a clear message
/// instead of silently treating either as a relative local path.
pub(super) fn unsupported_shorthand_message(input: &str) -> Option<&'static str> {
    let looks_like_unc = input.starts_with("\\\\")
        || ["smb:", "SMB:"].iter().any(|prefix| {
            input
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with("\\\\"))
        });
    // A bare `//host/share` has no scheme, so it is not a valid URI (unlike
    // `smb://host/share`, which `is_uri_like` already accepts untouched).
    let looks_like_bare_network_shorthand = input.starts_with("//") && !is_uri_like(input);
    let unsupported =
        looks_like_unc || looks_like_bare_network_shorthand || looks_like_scp_shorthand(input);
    if unsupported {
        Some(
            "UNC paths (\\\\host\\share) and SCP-style addresses (user@host:path) aren't \
             supported. Use a URI instead, such as smb://host/share, sftp://host/path, \
             ftp://host/path, or dav://host/path.",
        )
    } else {
        None
    }
}

pub(super) fn looks_like_scp_shorthand(input: &str) -> bool {
    if is_uri_like(input) {
        return false;
    }
    let Some((_user, after_at)) = input.split_once('@') else {
        return false;
    };
    let Some(host) = after_at.split(':').next() else {
        return false;
    };
    !host.is_empty() && after_at.contains(':') && !host.contains('/') && !host.contains('\\')
}

/// Joins natively, so a non-UTF-8 base stays byte-exact, and resolves `.` and
/// `..` as text.
pub(super) fn join_relative(base: &Path, relative: &str) -> PathBuf {
    lexically_normalized(&base.join(relative))
}

pub(super) fn is_relative_path_input(input: &str) -> bool {
    !input.starts_with(['/', '~']) && !is_uri_like(input) && virtual_root(input).is_none()
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
