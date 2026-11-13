// SPDX-License-Identifier: MIT

use strata_core::{
    i18n::{file_size, tr},
    model::{EntryKind, FileEntry, Location, MetadataValue},
    util::{DateFormat, set_date_format},
};
use strata_fs::services::{EntryType, entry_type};

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
    let empty_folder = Facts {
        child_count: Some(0),
        ..Facts::default()
    };
    let size = file_size(1_500);
    let cases: [(&str, bool, &Facts, &str, &str); 4] = [
        ("a file", false, &file, &size, "-rw-r--r--  644"),
        ("an empty folder", true, &empty_folder, "No items", ""),
        (
            "a folder whose count is unknown",
            true,
            &Facts::default(),
            "",
            "",
        ),
        (
            "a file the browser knows nothing of",
            false,
            &Facts::default(),
            "",
            "",
        ),
    ];
    for (case, folder, facts, size, mode) in cases {
        assert_eq!(size_cell(folder, facts), size, "{case}: size");
        assert_eq!(mode_cell(facts), mode, "{case}: mode");
    }
}

/// `strata_core::util::modified_date`: a dash for a date the browser does not know, else the
/// date in the saved format, here ISO 8601 for a fixed output (in the local time zone, so
/// only the day can differ).
#[test]
fn the_modified_cell_shows_the_date_in_the_saved_format() {
    assert_eq!(modified_cell(&Facts::default()), "—");
    set_date_format(DateFormat::Iso8601);
    // 2001-09-09 01:46:40 UTC.
    let known = Facts {
        modified: Some(1_000_000_000),
        ..Facts::default()
    };
    let shown = modified_cell(&known);
    assert!(
        shown.starts_with("2001-09-0") && shown.len() == 16 && shown.as_bytes()[10] == b' ',
        "{shown}"
    );
}

/// GTK's `entry_type`: a folder, a broken link and an entry of no known kind read their kind's
/// name from the catalogs; a file the MIME table knows reads its description, as strata-fs's
/// `entry_type` gives it for the same entry.
#[test]
fn the_type_cell_names_a_kind_from_the_catalogs_and_a_file_by_its_mime_type() {
    let folder = Location::local("/root");
    for (kind, name, expected) in [
        (EntryKind::Directory, "docs", EntryType::Folder),
        (EntryKind::SymbolicLink, "gone", EntryType::BrokenLink),
        (EntryKind::Other, "socket", EntryType::Other),
    ] {
        assert_eq!(
            type_cell(&folder, kind, name),
            tr(expected.description()),
            "{name}"
        );
    }
    let EntryType::Known(description) = entry_type(&file(&folder, "notes.txt")) else {
        panic!("the MIME table knows a text file");
    };
    assert_eq!(
        type_cell(&folder, EntryKind::File, "notes.txt"),
        description
    );
}

/// A file named `name` in `folder`, as a listing reads it, with nothing else known.
fn file(folder: &Location, name: &str) -> FileEntry {
    let path = folder.native_path().expect("a local folder").join(name);
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}
