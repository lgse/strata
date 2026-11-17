// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, path::Path};

use super::*;

#[test]
fn safe_filename_rejects_empty_slashes_dots_and_nul() {
    assert!(safe_filename(OsStr::new("report.png")));
    for bad in ["", ".", "..", "a/b", "a\0b"] {
        assert!(!safe_filename(OsStr::new(bad)), "expected unsafe: {bad:?}");
    }
}

#[test]
fn validate_filters_and_choices_enforce_limits_and_glob_rules() {
    let ok = PortalFileFilter {
        label: "Images".to_owned(),
        rules: vec![
            FilterRule::Pattern("*.png".to_owned()),
            FilterRule::MimeType("image/png".to_owned()),
        ],
    };
    assert!(validate_filters(&[ok], None).is_ok());

    let too_many_stars = PortalFileFilter {
        label: "Bad".to_owned(),
        rules: vec![FilterRule::Pattern("*a*b*c*".to_owned())],
    };
    assert!(matches!(
        validate_filters(&[too_many_stars], None),
        Err(PortalError::InvalidArgument(_))
    ));

    let valid_choice = PortalChoice {
        id: "encoding".to_owned(),
        label: "Encoding".to_owned(),
        pairs: vec![("utf8".to_owned(), "UTF-8".to_owned())],
        initial_selection: "utf8".to_owned(),
    };
    assert!(validate_common_request("Choose", Some("Open"), None, &[valid_choice]).is_ok());
}

#[test]
fn hyprland_window_size_matches_single_mapped_visible_client() {
    let valid =
        br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960,1000]}]"#;
    assert_eq!(
        window_geometry::application_window_size(valid, "ZEN"),
        Some((960, 1000))
    );

    let duplicate = br#"[
        {"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960,1000]},
        {"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[800,600]}
    ]"#;
    assert_eq!(
        window_geometry::application_window_size(duplicate, "zen"),
        None
    );
    assert_eq!(
        window_geometry::socket_path(Path::new("/run/user/1000"), "../bad"),
        None
    );
}
