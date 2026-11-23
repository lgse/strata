// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn package_metadata_is_available_to_the_about_page() {
    assert_eq!(REPOSITORY, "https://github.com/LGSE/strata");
    assert_eq!(AUTHOR, "LGSE Ltd.");
}

#[test]
fn default_release_tag_parses_to_a_version() {
    assert!(
        crate::services::Version::parse(RELEASE_TAG).is_some(),
        "expected {RELEASE_TAG:?} to parse as a Version"
    );
}

#[test]
fn developer_build_reports_stable_build_kind() {
    assert_eq!(build_kind(), BuildKind::Stable);
}

#[test]
fn installed_version_agrees_with_version_for_a_default_build() {
    assert_eq!(installed_version().to_string(), VERSION);
}
