// SPDX-License-Identifier: MIT

use super::*;

/// Ascending type order, which puts folders first with or without the preference.
const TYPE_ORDER_FOLDERS_FIRST: [&str; 8] = [
    "folder_a",
    "folder_z",
    "a_notes.json",
    "b_notes.json",
    "doc.pdf",
    "script.py",
    "a_other.qqqqq",
    "z_other.qqqqq",
];

const TYPES_DESCENDING_FOLDERS_FIRST: [&str; 8] = [
    "folder_a",
    "folder_z",
    "a_other.qqqqq",
    "z_other.qqqqq",
    "script.py",
    "doc.pdf",
    "a_notes.json",
    "b_notes.json",
];

const TYPES_DESCENDING_FOLDERS_LAST: [&str; 8] = [
    "a_other.qqqqq",
    "z_other.qqqqq",
    "script.py",
    "doc.pdf",
    "a_notes.json",
    "b_notes.json",
    "folder_a",
    "folder_z",
];

fn typed_entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        kind,
        ..child("/fixture", name)
    }
}

fn mixed_types() -> Vec<FileEntry> {
    vec![
        typed_entry("z_other.qqqqq", EntryKind::File),
        typed_entry("b_notes.json", EntryKind::File),
        typed_entry("a_notes.json", EntryKind::File),
        typed_entry("folder_z", EntryKind::Directory),
        typed_entry("folder_a", EntryKind::Directory),
        typed_entry("a_other.qqqqq", EntryKind::File),
        typed_entry("doc.pdf", EntryKind::File),
        typed_entry("script.py", EntryKind::File),
    ]
}

/// Sorts the mixed types ascending and then, from that order, descending.
fn assert_type_order(folders_first: bool, ascending: [&str; 8], descending: [&str; 8]) {
    let mut entries = mixed_types();
    for (sort_direction, expected) in [
        (SortDirection::Ascending, ascending),
        (SortDirection::Descending, descending),
    ] {
        let preferences = ViewPreferences {
            name_comparison: crate::model::NameComparison::default(),
            folders_first,
            sort_key: SortKey::Type,
            sort_direction,
            show_hidden: false,
        };
        entries.sort_by(|left, right| compare_entries(left, right, preferences));
        assert_eq!(names(&entries), expected, "{sort_direction:?}");
    }
}

/// Entries whose kinds, sizes and times cycle through every metadata availability.
fn every_metadata_shape() -> Vec<FileEntry> {
    [
        "ß.txt",
        "ss.txt",
        "İ.txt",
        "i.txt",
        "中文10.txt",
        "中文2.txt",
        "FILE.txt",
        "file.txt",
        "file02.txt",
        "file2.txt",
        "café.png",
        "über.rs",
        "unknown.strata-unknown-extension",
        "same.txt",
        "same.txt",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| metadata_shape(index, name))
    .collect()
}

fn metadata_shape(index: usize, name: &str) -> FileEntry {
    let modified = match index % 3 {
        0 => MetadataValue::Known(20),
        1 => MetadataValue::Known(10),
        _ => MetadataValue::Unavailable,
    };
    FileEntry {
        kind: if index.is_multiple_of(4) {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: match index % 4 {
            0 => MetadataValue::Unknown,
            1 => MetadataValue::Unavailable,
            _ => MetadataValue::Known(10),
        },
        created_unix_seconds: modified.clone(),
        recent_unix_seconds: modified.clone(),
        modified_unix_seconds: modified,
        ..file_entry(&format!("/fixture/{index}/{name}"), name)
    }
}

fn every_sort_preference() -> Vec<ViewPreferences> {
    let mut preferences = Vec::new();
    for sort_key in [
        SortKey::Name,
        SortKey::Type,
        SortKey::Size,
        SortKey::Modified,
        SortKey::Extension,
        SortKey::Created,
        SortKey::Unsorted,
        SortKey::Recency,
        SortKey::DeviceOrder,
    ] {
        for sort_direction in [SortDirection::Ascending, SortDirection::Descending] {
            for name_comparison in [
                crate::model::NameComparison::default(),
                crate::model::NameComparison {
                    natural: false,
                    case_sensitive: false,
                },
                crate::model::NameComparison {
                    natural: true,
                    case_sensitive: true,
                },
                crate::model::NameComparison {
                    natural: false,
                    case_sensitive: true,
                },
            ] {
                preferences.extend([false, true].map(|folders_first| ViewPreferences {
                    sort_key,
                    sort_direction,
                    folders_first,
                    name_comparison,
                    ..ViewPreferences::default()
                }));
            }
        }
    }
    preferences
}

/// Merges `entries` four at a time, asserting each batch's insertions replay into the merge.
fn merged_in_batches(entries: &[FileEntry], preferences: ViewPreferences) -> Vec<FileEntry> {
    let mut merged = Vec::new();
    for batch in entries.chunks(4) {
        let mut replayed = merged.clone();
        let (next, insertions) = super::super::merge_entries(merged, batch.to_vec(), preferences);
        for insertion in insertions {
            replayed.splice(insertion.position..insertion.position, insertion.entries);
        }
        assert_eq!(replayed, next, "{preferences:?}");
        merged = next;
    }
    merged
}

fn inserted_one_by_one(entries: &[FileEntry], preferences: ViewPreferences) -> Vec<FileEntry> {
    let mut monitored = Vec::new();
    let mut splices = Vec::new();
    for entry in entries {
        super::super::insert_monitored_entry(
            &mut monitored,
            entry.clone(),
            preferences,
            &mut splices,
        );
    }
    monitored
}

#[test]
fn batches_are_merged_into_one_global_sort_order() {
    let mut state = loaded("/fixture", vec![child("/fixture", "z")]);
    assert!(state.select(0, 0));

    let (_, insertions) = state
        .apply_batch(RequestId(1), vec![child("/fixture", "a")])
        .expect("the request is current");

    let positions: Vec<_> = insertions
        .iter()
        .map(|insertion| insertion.position)
        .collect();
    assert_eq!(positions, [0]);
    assert_eq!(column_names(&state, 0), ["a", "z"]);
    assert_eq!(state.columns[0].selected, Some(1));
}

#[test]
fn names_are_sorted_case_insensitively() {
    let state = loaded(
        "/fixture",
        children("/fixture", &["apple", "Banana", "cherry", "Date"]),
    );

    assert_eq!(
        column_names(&state, 0),
        ["apple", "Banana", "cherry", "Date"]
    );
}

#[test]
fn names_that_differ_only_by_case_have_a_deterministic_order() {
    assert_eq!(compare_display_names("file", "FILE"), Ordering::Greater);
    assert_eq!(
        compare_display_names("Straße", "STRASSE"),
        Ordering::Greater
    );
}

#[test]
fn numeric_suffixes_sort_naturally() {
    for (left, right) in [
        ("File 1", "File 2"),
        ("File 2", "File 10"),
        ("File 1", "File 10"),
        ("File 99999999999999999999", "File 100000000000000000000"),
        ("File 0002", "File 10"),
        ("File 02", "File 2"),
        ("File 0", "File 00"),
        ("file 2 part 9", "File 2 part 10"),
        ("Straße 2", "STRASSE 10"),
        ("File 2", "File 2a"),
    ] {
        assert_eq!(
            compare_display_names(left, right),
            Ordering::Less,
            "{left} < {right}"
        );
        assert_eq!(
            compare_display_names(right, left),
            Ordering::Greater,
            "{right} > {left}"
        );
        assert_eq!(compare_display_names(left, left), Ordering::Equal);
    }
}

#[test]
fn changing_sort_preferences_preserves_the_selected_entry() {
    let mut state = loaded("/fixture", children("/fixture", &["a", "z"]));
    assert!(state.select(0, 0));

    assert!(state.apply_sort_preferences(0, descending()).is_some());

    assert_eq!(
        (column_names(&state, 0), state.columns[0].selected),
        (owned(&["z", "a"]), Some(1))
    );
    assert_eq!(focused_name(&state), Some("a".into()));
}

#[test]
fn changing_sort_preferences_only_reorders_the_target_column() {
    let mut state = loaded("/fixture", children("/fixture", &["a", "z"]));
    assert!(state.descend(0, location("/fixture/child"), RequestId(2)));
    state.apply_batch(RequestId(2), children("/fixture/child", &["a", "z"]));

    assert!(state.apply_sort_preferences(1, descending()).is_some());

    assert_eq!(state.columns[0].entries[0].display_name, "a");
    assert_eq!(state.columns[1].entries[0].display_name, "z");
}

#[test]
fn recency_sort_uses_existing_metadata_availability_order() {
    let recent = |name: &str, recent_unix_seconds| FileEntry {
        recent_unix_seconds,
        ..file_entry(&format!("/fixture/{name}"), name)
    };
    let mut entries = [
        file_entry("/fixture/unknown", "unknown"),
        recent("unavailable", MetadataValue::Unavailable),
        recent("older", MetadataValue::Known(10)),
        recent("newest", MetadataValue::Known(20)),
    ];

    entries.sort_by(|left, right| {
        compare_entries(
            left,
            right,
            ViewPreferences {
                name_comparison: crate::model::NameComparison::default(),
                sort_key: SortKey::Recency,
                sort_direction: SortDirection::Descending,
                folders_first: false,
                show_hidden: false,
            },
        )
    });

    assert_eq!(
        names(&entries),
        ["unavailable", "unknown", "newest", "older"]
    );
}

#[test]
fn type_sorting_orders_by_mime_descriptions_with_folders_first() {
    assert_type_order(
        true,
        TYPE_ORDER_FOLDERS_FIRST,
        TYPES_DESCENDING_FOLDERS_FIRST,
    );
}

#[test]
fn type_sorting_orders_by_mime_descriptions_without_folders_first() {
    assert_type_order(
        false,
        TYPE_ORDER_FOLDERS_FIRST,
        TYPES_DESCENDING_FOLDERS_LAST,
    );
}

#[test]
fn precomputed_sort_preserves_natural_utf8_order() {
    let entries: Vec<FileEntry> = [
        "über_10.txt",
        "über_2.txt",
        "Straße_1.txt",
        "STRASSE_2.txt",
        "café_latte.txt",
        "café.txt",
        "apple.txt",
        "Banana.txt",
    ]
    .into_iter()
    .map(|name| file_entry(&format!("/test/{name}"), name))
    .collect();

    let sorted = super::super::sort_entries(entries, ViewPreferences::default());

    assert_eq!(
        names(&sorted),
        [
            "apple.txt",
            "Banana.txt",
            "café.txt",
            "café_latte.txt",
            "Straße_1.txt",
            "STRASSE_2.txt",
            "über_2.txt",
            "über_10.txt",
        ]
    );
}

#[test]
fn keyed_sort_and_batch_merge_match_monitor_order() {
    let entries = every_metadata_shape();
    for preferences in every_sort_preference() {
        let mut expected = entries.clone();
        expected.sort_by(|left, right| compare_entries(left, right, preferences));
        assert_eq!(
            super::super::sort_entries(entries.clone(), preferences),
            expected,
            "{preferences:?}"
        );
        assert_eq!(
            merged_in_batches(&entries, preferences),
            expected,
            "{preferences:?}"
        );
        assert_eq!(
            inserted_one_by_one(&entries, preferences),
            expected,
            "{preferences:?}"
        );
    }
}

#[test]
fn configured_name_comparison_orders_numbers_case_and_unicode() {
    use crate::model::NameComparison;
    for (comparison, input, expected) in [
        (
            NameComparison::default(),
            vec!["file10", "File2", "file1"],
            vec!["file1", "File2", "file10"],
        ),
        (
            NameComparison {
                natural: false,
                case_sensitive: false,
            },
            vec!["file2", "file10", "file1"],
            vec!["file1", "file10", "file2"],
        ),
        (
            NameComparison {
                natural: true,
                case_sensitive: true,
            },
            vec!["a2", "Z10", "a10", "Z2"],
            vec!["Z2", "Z10", "a2", "a10"],
        ),
        (
            NameComparison {
                natural: false,
                case_sensitive: false,
            },
            vec!["über2", "ÜBER10"],
            vec!["ÜBER10", "über2"],
        ),
    ] {
        let entries = input
            .iter()
            .map(|name| typed_entry(name, EntryKind::File))
            .collect();
        let prefs = ViewPreferences {
            name_comparison: comparison,
            ..ViewPreferences::default()
        };
        assert_eq!(names(&super::super::sort_entries(entries, prefs)), expected);
    }
}

#[test]
fn extension_sort_uses_final_suffix_and_name_comparison_for_ties() {
    let entries = [
        "z.rs",
        "file2.md",
        "file10.md",
        "archive.tar.gz",
        ".gitignore",
        "readme",
    ]
    .map(|name| typed_entry(name, EntryKind::File))
    .to_vec();
    let prefs = ViewPreferences {
        sort_key: SortKey::Extension,
        ..ViewPreferences::default()
    };
    assert_eq!(
        names(&super::super::sort_entries(entries.clone(), prefs)),
        [
            ".gitignore",
            "readme",
            "archive.tar.gz",
            "file2.md",
            "file10.md",
            "z.rs"
        ]
    );
    let alphabetical = ViewPreferences {
        name_comparison: crate::model::NameComparison {
            natural: false,
            case_sensitive: false,
        },
        ..prefs
    };
    assert_eq!(
        names(&super::super::sort_entries(entries, alphabetical)),
        [
            ".gitignore",
            "readme",
            "archive.tar.gz",
            "file10.md",
            "file2.md",
            "z.rs"
        ]
    );
}

#[test]
fn created_sort_keeps_missing_times_last_in_both_directions() {
    let entries = [
        ("unknown", MetadataValue::Unknown),
        ("new", MetadataValue::Known(20)),
        ("unavailable", MetadataValue::Unavailable),
        ("old", MetadataValue::Known(10)),
    ]
    .into_iter()
    .map(|(name, created)| FileEntry {
        created_unix_seconds: created,
        ..typed_entry(name, EntryKind::File)
    })
    .collect::<Vec<_>>();
    for (sort_direction, expected) in [
        (
            SortDirection::Ascending,
            ["old", "new", "unknown", "unavailable"],
        ),
        (
            SortDirection::Descending,
            ["new", "old", "unknown", "unavailable"],
        ),
    ] {
        let prefs = ViewPreferences {
            sort_key: SortKey::Created,
            sort_direction,
            ..ViewPreferences::default()
        };
        assert_eq!(
            names(&super::super::sort_entries(entries.clone(), prefs)),
            expected
        );
    }
}

#[test]
fn unsorted_preserves_discovery_within_groups_across_batches_and_monitoring() {
    let entries = vec![
        typed_entry("z.txt", EntryKind::File),
        typed_entry("z-dir", EntryKind::Directory),
        typed_entry("a.txt", EntryKind::File),
        typed_entry("a-dir", EntryKind::Directory),
        typed_entry("last.txt", EntryKind::File),
    ];
    for (folders_first, expected) in [
        (false, vec!["z.txt", "z-dir", "a.txt", "a-dir", "last.txt"]),
        (true, vec!["z-dir", "a-dir", "z.txt", "a.txt", "last.txt"]),
    ] {
        let prefs = ViewPreferences {
            sort_key: SortKey::Unsorted,
            folders_first,
            sort_direction: SortDirection::Descending,
            ..ViewPreferences::default()
        };
        assert_eq!(
            names(&super::super::sort_entries(entries.clone(), prefs)),
            expected
        );
        assert_eq!(names(&merged_in_batches(&entries, prefs)), expected);
        assert_eq!(names(&inserted_one_by_one(&entries, prefs)), expected);
    }
}
