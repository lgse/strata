// SPDX-License-Identifier: MIT

use std::fs;

use super::{CornerRoundness, default_corner_roundness, default_corner_roundness_for};
use crate::settings::{Preferences, parse_preferences};

struct OsMarkerCase {
    name: &'static str,
    os_release: Option<&'static str>,
    system_version: bool,
    user_version: bool,
    expected: CornerRoundness,
}

const OS_MARKER_CASES: &[OsMarkerCase] = &[
    OsMarkerCase {
        name: "os-release unquoted omarchy",
        os_release: Some("NAME=Omarchy\nID=omarchy\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::SQUARE,
    },
    OsMarkerCase {
        name: "os-release double-quoted omarchy",
        os_release: Some("ID=\"omarchy\"\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::SQUARE,
    },
    OsMarkerCase {
        name: "os-release single-quoted omarchy",
        os_release: Some("ID='omarchy'\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::SQUARE,
    },
    OsMarkerCase {
        name: "os-release mixed-case omarchy",
        os_release: Some("ID=\"Omarchy\"\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::SQUARE,
    },
    OsMarkerCase {
        name: "system omarchy version file",
        os_release: Some("ID=arch\n"),
        system_version: true,
        user_version: false,
        expected: CornerRoundness::SQUARE,
    },
    OsMarkerCase {
        name: "user omarchy version file",
        os_release: Some("ID=arch\n"),
        system_version: false,
        user_version: true,
        expected: CornerRoundness::SQUARE,
    },
    OsMarkerCase {
        name: "plain arch",
        os_release: Some("NAME=\"Arch Linux\"\nID=arch\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::DEFAULT,
    },
    OsMarkerCase {
        name: "plain ubuntu",
        os_release: Some("NAME=\"Ubuntu\"\nID=ubuntu\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::DEFAULT,
    },
    OsMarkerCase {
        name: "plain fedora",
        os_release: Some("NAME=\"Fedora Linux\"\nID=fedora\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::DEFAULT,
    },
    OsMarkerCase {
        name: "id_like omarchy does not match",
        os_release: Some("ID=arch\nID_LIKE=omarchy\n"),
        system_version: false,
        user_version: false,
        expected: CornerRoundness::DEFAULT,
    },
    OsMarkerCase {
        name: "missing files",
        os_release: None,
        system_version: false,
        user_version: false,
        expected: CornerRoundness::DEFAULT,
    },
];

#[test]
fn default_corner_roundness_detects_omarchy_markers() {
    for case in OS_MARKER_CASES {
        let dir = tempfile::tempdir().expect("temp dir for os markers");
        let os_release = dir.path().join("os-release");
        let system_version = dir.path().join("system-version");
        let user_version = dir.path().join("user-version");

        if let Some(contents) = case.os_release {
            fs::write(&os_release, contents).expect("write os-release");
        }
        if case.system_version {
            fs::write(&system_version, "4.0.0\n").expect("write system version");
        }
        if case.user_version {
            fs::write(&user_version, "3.2.0\n").expect("write user version");
        }

        let actual =
            default_corner_roundness_for(&os_release, &system_version, Some(&user_version));
        assert_eq!(actual, case.expected, "{}", case.name);
    }

    let missing = tempfile::tempdir().expect("temp dir for missing user version");
    assert_eq!(
        default_corner_roundness_for(
            &missing.path().join("os-release"),
            &missing.path().join("system-version"),
            None,
        ),
        CornerRoundness::DEFAULT,
        "missing user version path"
    );
}

#[test]
fn corner_roundness_deserializes_explicit_values_and_salvages_invalid_types() {
    assert_eq!(CornerRoundness::default(), CornerRoundness::DEFAULT);
    assert_eq!(CornerRoundness::SQUARE.percent(), 0);
    assert_eq!(
        Preferences::default().corner_roundness,
        default_corner_roundness()
    );

    let omitted =
        parse_preferences("mode = \"theme\"\ntheme = \"tokyo-night\"\n").expect("parse omitted");
    assert_eq!(omitted.corner_roundness, default_corner_roundness());

    for (raw, expected) in [
        ("-20", 0),
        ("0", 0),
        ("60", 60),
        ("100", 100),
        ("200", 200),
        ("999", CornerRoundness::MAX),
    ] {
        let toml = format!("mode = \"theme\"\ntheme = \"tokyo-night\"\ncorner_roundness = {raw}\n");
        let parsed = parse_preferences(&toml).expect("parse explicit corner_roundness");
        assert_eq!(parsed.corner_roundness.percent(), expected, "raw={raw}");
    }

    let salvaged = parse_preferences(
        "mode = \"theme\"\ntheme = \"tokyo-night\"\ncorner_roundness = \"rounded\"\n",
    )
    .expect("salvage invalid corner_roundness");
    assert_eq!(salvaged.corner_roundness, default_corner_roundness());
}
