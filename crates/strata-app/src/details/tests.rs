// SPDX-License-Identifier: MIT

use strata_core::{
    i18n::tr,
    model::{EntryKind, Location},
};
use strata_fs::services::EntryType;

use super::{duration, icons_line, mode_cell, modified_cell, permissions, size_cell, type_cell};
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
        ..Facts::default()
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

/// GTK's `format_permissions`: the kind, nine bits as letters, then the octal mode.
#[test]
fn permissions_read_as_the_list_views_mode_column_shows_them() {
    for (mode, expected) in [
        (0o040_755, "drwxr-xr-x  755"),
        (0o100_644, "-rw-r--r--  644"),
        (0o100_600, "-rw-------  600"),
        (0o100_777, "-rwxrwxrwx  777"),
    ] {
        assert_eq!(permissions(mode), expected, "{mode:o}");
    }
}

/// GTK's `entry_size` and `entry_mode`: a folder shows its item count, a file its size, and
/// what the browser does not know shows nothing.
#[test]
fn the_size_and_mode_cells_show_what_the_browser_knows() {
    let file = Facts {
        size: Some(1_500),
        mode: Some(0o100_644),
        ..Facts::default()
    };
    let folder = Facts {
        child_count: Some(0),
        ..Facts::default()
    };
    assert_eq!(size_cell(false, &file), strata_core::i18n::file_size(1_500));
    assert_eq!(size_cell(true, &folder), "No items");
    assert_eq!(size_cell(false, &Facts::default()), "");
    assert_eq!(mode_cell(&file), "-rw-r--r--  644");
    assert_eq!(mode_cell(&Facts::default()), "");
    assert_eq!(modified_cell(&Facts::default()), "—");
}

/// GTK's `entry_type`: a folder and an entry of no known kind read their kind's name from the
/// catalogs, not a MIME description.
#[test]
fn the_type_cell_names_a_folder_and_an_unknown_kind_from_the_catalogs() {
    let folder = Location::local("/root");
    for (kind, name, expected) in [
        (EntryKind::Directory, "docs", EntryType::Folder),
        (EntryKind::Other, "socket", EntryType::Other),
    ] {
        assert_eq!(
            type_cell(&folder, kind, name),
            tr(expected.description()),
            "{name}"
        );
    }
}
