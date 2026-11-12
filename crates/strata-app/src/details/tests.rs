// SPDX-License-Identifier: MIT

use super::{duration, icons_line};
use crate::listing::Facts;

/// GTK's `format_duration`: minutes and seconds under an hour, hours on from one.
#[test]
fn a_duration_reads_as_a_clock() {
    for (seconds, expected) in [
        (0, "0:00"),
        (3, "0:03"),
        (59, "0:59"),
        (60, "1:00"),
        (3_599, "59:59"),
        (3_600, "1:00:00"),
        (3_661, "1:01:01"),
        (36_000, "10:00:00"),
    ] {
        assert_eq!(duration(seconds), expected, "{seconds} s");
    }
}

/// GTK's `entry_icons_item_info`: a folder shows its item count only; a file the first the
/// browser knows of its duration, its dimensions and its size.
#[test]
fn the_icons_line_shows_the_first_fact_known() {
    let everything = Facts {
        size: Some(1_500),
        child_count: Some(3),
        duration_seconds: Some(2),
        dimensions: Some((320, 180)),
    };
    let cases = [
        (
            "an empty folder",
            true,
            Facts {
                child_count: Some(0),
                ..Facts::default()
            },
            Some("No items"),
        ),
        (
            "a folder of one",
            true,
            Facts {
                child_count: Some(1),
                ..Facts::default()
            },
            Some("1 item"),
        ),
        ("a folder of three", true, everything, Some("3 items")),
        (
            "a folder not yet counted",
            true,
            Facts {
                size: Some(4_096),
                ..Facts::default()
            },
            None,
        ),
        ("a video", false, everything, Some("0:02")),
        (
            "an image",
            false,
            Facts {
                duration_seconds: None,
                ..everything
            },
            Some("320×180"),
        ),
        (
            "a file",
            false,
            Facts {
                size: Some(1_500),
                ..Facts::default()
            },
            Some("1.5 kB"),
        ),
        ("a file not yet read", false, Facts::default(), None),
    ];
    for (name, folder, facts, expected) in cases {
        assert_eq!(icons_line(folder, &facts).as_deref(), expected, "{name}");
    }
}
