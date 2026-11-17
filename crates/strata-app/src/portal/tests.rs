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
    for (reply, why) in [
        (
            &br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":true,"size":[960,1000]}]"#[..],
            "a hidden client",
        ),
        (
            br#"[{"class":"zen","initialClass":"zen","mapped":false,"hidden":false,"size":[960,1000]}]"#,
            "an unmapped client",
        ),
        (
            br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[0,1000]}]"#,
            "a client with no width",
        ),
        (
            br#"[{"class":"zen","mapped":true,"hidden":false,"size":[960,1000]}]"#,
            "a client without an initial class",
        ),
        (
            br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960]}]"#,
            "a size that is not a pair",
        ),
        (
            br#"[{"class":"x","initialClass":"x","mapped":1,"hidden":false,"size":[9,9]},{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960,1000]}]"#,
            "a reply with a malformed client",
        ),
        (br#"{"class":"zen"}"#, "a reply that is not a list"),
    ] {
        assert_eq!(
            window_geometry::application_window_size(reply, "zen"),
            None,
            "{why}"
        );
    }
    let by_initial_class = br#"[{"class":"other","initialClass":"Zen","mapped":true,"hidden":false,"size":[800,600]}]"#;
    assert_eq!(
        window_geometry::application_window_size(by_initial_class, "zen"),
        Some((800, 600))
    );
}

#[test]
fn hyprland_socket_needs_an_absolute_runtime_and_a_plain_signature() {
    let runtime = Path::new("/run/user/1000");
    assert_eq!(
        window_geometry::socket_path(runtime, "abc_12-x"),
        Some(runtime.join("hypr/abc_12-x/.socket.sock"))
    );
    for (runtime, signature) in [
        ("/run/user/1000", "../bad"),
        ("/run/user/1000", ""),
        ("run/user/1000", "abc"),
    ] {
        assert_eq!(
            window_geometry::socket_path(Path::new(runtime), signature),
            None,
            "{runtime:?} {signature:?}"
        );
    }
}
