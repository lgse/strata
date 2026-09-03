// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn package_metadata_is_available_to_the_about_page() {
    assert!(!DESCRIPTION.is_empty());
    assert!(!VERSION.is_empty());
    assert!(!COMMIT.is_empty());
    assert_eq!(REPOSITORY, "https://github.com/LGSE/strata");
    assert_eq!(AUTHOR, "LGSE Ltd.");
}

/// The default `RELEASE_TAG` injected by `build.rs` (no `STRATA_RELEASE_TAG`
/// override, i.e. every developer build and this test run) must be a
/// well-formed tag the release-channel grammar accepts.
#[test]
fn default_release_tag_parses_to_a_version() {
    assert!(
        crate::services::Version::parse(RELEASE_TAG).is_some(),
        "expected {RELEASE_TAG:?} to parse as a Version"
    );
}

/// A developer build has no `STRATA_BUILD_KIND` override, so it must
/// report itself as `Stable` -- the fallback that keeps a build unable to
/// identify itself from silently claiming to be a preview.
#[test]
fn developer_build_reports_stable_build_kind() {
    assert_eq!(build_kind(), BuildKind::Stable);
}

/// For a default build, `installed_version()` parses `RELEASE_TAG`
/// (`v{CARGO_PKG_VERSION}`), which must agree with `VERSION`
/// (`CARGO_PKG_VERSION` itself) once rendered back out.
#[test]
fn installed_version_agrees_with_version_for_a_default_build() {
    assert_eq!(installed_version().to_string(), VERSION);
}

/// `installed_version()`'s fallback chain reads `RELEASE_TAG` and `VERSION`
/// as `env!`-injected constants, not parameters, so this build's default
/// values can't be swapped out to actually drive the function down its
/// fallback branches from a test. Instead this exercises the building
/// blocks the chain depends on: `Version::parse` must reject the malformed
/// and empty inputs that would trigger a fallback, and the `"0.0.0"` floor
/// the chain lands on if every real parse fails must itself always parse
/// (this is what makes `installed_version()`'s final `.unwrap_or_else`
/// branch panic-free).
#[test]
fn version_parse_rejects_invalid_input_and_accepts_the_fallback_floor() {
    assert!(Version::parse("not-a-version").is_none());
    assert!(Version::parse("").is_none());
    // The floor of the fallback chain must itself always parse.
    assert_eq!(
        Version::parse("0.0.0").map(|v| v.to_string()),
        Some("0.0.0".to_owned())
    );
}
