// SPDX-License-Identifier: GPL-3.0-or-later

use crate::services::{BuildKind, Version};

pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("STRATA_BUILD_COMMIT");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");

/// The release tag this build was published under, e.g. `v0.5.0` or
/// `v0.5.0-rc.1`.
///
/// Injected by `build.rs`, mirroring how it injects [`COMMIT`]. Defaults to
/// `v{CARGO_PKG_VERSION}` when `STRATA_RELEASE_TAG` is unset or empty --
/// i.e. every developer build -- so this is where prerelease identity
/// travels without ever bumping `Cargo.toml` for an RC (see D3 in the
/// release-channel design notes).
pub const RELEASE_TAG: &str = env!("STRATA_RELEASE_TAG");

/// The build kind this binary was published as, as the raw string
/// `build.rs` injected: `"stable"`, `"alpha"`, `"beta"`, `"rc"`, or
/// `"nightly"`.
///
/// Defaults to `"stable"` when `STRATA_BUILD_KIND` is unset, empty, or not
/// one of those three values. Prefer [`build_kind`] over reading this
/// directly -- it gives the parsed, comparable [`BuildKind`].
pub const BUILD_KIND: &str = env!("STRATA_BUILD_KIND");

/// Parses [`BUILD_KIND`], falling back to [`BuildKind::Stable`] for
/// anything unrecognised.
///
/// This must fail closed, the same way [`crate::services::Channel::parse`]
/// does: a build that cannot identify its own kind must present as an
/// ordinary stable build, never as a preview.
pub fn build_kind() -> BuildKind {
    match BUILD_KIND {
        "nightly" => BuildKind::Nightly,
        "alpha" => BuildKind::Alpha,
        "beta" => BuildKind::Beta,
        "rc" => BuildKind::Rc,
        _ => BuildKind::Stable,
    }
}

/// The version this build identifies as, for update-channel comparisons.
///
/// Parses [`RELEASE_TAG`] first, since that -- not `Cargo.toml` -- is where
/// prerelease identity travels (D3). Falls back to parsing [`VERSION`] if
/// `RELEASE_TAG` somehow fails to parse, and to `0.0.0` if that fails too:
/// a build that cannot identify itself must not crash the updater, and the
/// `0.0.0` floor means every real release still sorts newer, so a
/// misconfigured build always looks eligible for the next real update
/// rather than silently blocking it.
///
/// Never panics and never `.unwrap()`s: the final fallback the chain lands
/// on is a fixed literal that will always parse and is covered by
/// `installed_version_fallback_chain_never_panics` (`build_info::tests`).
pub fn installed_version() -> Version {
    Version::parse(RELEASE_TAG)
        .or_else(|| Version::parse(VERSION))
        .unwrap_or_else(|| {
            Version::parse("0.0.0").expect(
                "\"0.0.0\" is exactly the bare MAJOR.MINOR.PATCH form Version::parse accepts",
            )
        })
}

#[cfg(test)]
mod tests;
