// SPDX-License-Identifier: MIT

use super::uri::is_uri_like;

/// UNC paths (`\\host\share`, bare `//host/share`) and SCP-style addresses
/// (`user@host:path`) are deliberately not accepted as location-bar shorthand
/// (see lgse/strata#20) so a proper URI (`smb://`, `sftp://`, ...) is always
/// preserved verbatim rather than being guessed at. Report a clear message
/// instead of silently treating either as a relative local path.
pub(in crate::app::browser) fn unsupported_shorthand_message(input: &str) -> Option<&'static str> {
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

fn looks_like_scp_shorthand(input: &str) -> bool {
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
