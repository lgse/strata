// SPDX-License-Identifier: MIT

use std::path::Path;

use strata_components::contract::{EntryKind, EntrySource};
use strata_core::model::{EntryKind as Kind, FileEntry, Location, MetadataValue, ViewPreferences};
use strata_fs::{app::compare_type_groups, services::entry_type};

use super::{Column, Edit, Row};
use crate::slots::Ids;

const FOLDER: &str = "/listing";

fn entry(name: &str) -> FileEntry {
    let kind = if name.ends_with('/') {
        Kind::Directory
    } else {
        Kind::File
    };
    let name = name.trim_end_matches('/');
    FileEntry {
        location: Location::local(Path::new(FOLDER).join(name)),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind,
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
        is_hidden: name.starts_with('.'),
    }
}

fn rows(ids: &Ids, names: &[&str]) -> Vec<Row> {
    names
        .iter()
        .map(|name| Row::of(&entry(name), ids))
        .collect()
}

fn column(ids: &Ids, names: &[&str]) -> Column {
    let mut column = Column::new(Location::local(FOLDER), false, None);
    column.apply(Edit::Replace(rows(ids, names)));
    column
}

fn shown(column: &Column) -> Vec<String> {
    (0..column.len())
        .map(|index| column.entry(index).name.to_owned())
        .collect()
}

/// Each shown row with the browser's position the view would send for it.
fn positions(column: &Column) -> Vec<(String, usize)> {
    (0..column.len())
        .map(|index| {
            let position = column.position_at(index).expect("a position");
            (column.entry(index).name.to_owned(), position)
        })
        .collect()
}

#[test]
fn an_edit_leaves_the_rows_the_browser_holds_and_shows_those_not_hidden() {
    type Case = (&'static str, fn(&Ids) -> Edit, &'static [&'static str]);
    const LISTED: &[&str] = &[".git/", "docs/", ".env", "a.txt", "b.txt"];
    let cases: [Case; 5] = [
        (
            "as listed",
            |_| Edit::Insert(Vec::new()),
            &["docs/", "a.txt", "b.txt"],
        ),
        (
            "a range published after the last row",
            |ids| Edit::Insert(vec![(5, rows(ids, &[".z", "c.txt"]))]),
            &["docs/", "a.txt", "b.txt", "c.txt"],
        ),
        (
            "two insertions, the second counted after the first",
            |ids| {
                Edit::Insert(vec![
                    (0, rows(ids, &["0.txt"])),
                    (4, rows(ids, &[".mid", "9.txt"])),
                ])
            },
            &["0.txt", "docs/", "9.txt", "a.txt", "b.txt"],
        ),
        (
            "a removal and a replacement",
            |ids| Edit::Splice(vec![(1, 1, Vec::new()), (2, 2, rows(ids, &["z.txt"]))]),
            &["z.txt"],
        ),
        (
            "a listing replaced",
            |ids| Edit::Replace(rows(ids, &["only.txt", ".only"])),
            &["only.txt"],
        ),
    ];
    for (case, edit, expected) in cases {
        let ids = Ids::default();
        let mut column = column(&ids, LISTED);
        let version = column.version;

        column.apply(edit(&ids));

        let names: Vec<_> = expected
            .iter()
            .map(|name| name.trim_end_matches('/'))
            .collect();
        assert_eq!(shown(&column), names, "{case}");
        for (name, position) in positions(&column) {
            let id = ids.of(&entry(&name).location);
            assert_eq!(column.position_of(id), Some(position), "{case}: {name}");
            let index = column.index_of(id).expect("shown");
            assert_eq!(column.entry(index).name, name, "{case}");
        }
        // The view reads a column's rows again only when its version moves.
        assert_eq!(column.version != version, case != "as listed", "{case}");
    }
}

#[test]
fn showing_hidden_entries_changes_what_shows_and_no_position() {
    let ids = Ids::default();
    let mut column = column(&ids, &[".git/", "docs/", ".env", "a.txt"]);
    let hidden = ids.of(&entry(".env").location);
    assert_eq!(
        positions(&column),
        [("docs".to_owned(), 1), ("a.txt".to_owned(), 3)]
    );
    assert_eq!(column.index_of(hidden), None);
    assert_eq!(
        column.position_of(hidden),
        Some(2),
        "held, though not shown"
    );

    column.set_show_hidden(true);

    assert_eq!(shown(&column), [".git", "docs", ".env", "a.txt"]);
    assert_eq!(column.position_at(2), Some(2));
    assert_eq!(column.index_of(hidden), Some(2));

    column.set_show_hidden(false);

    assert_eq!(
        positions(&column),
        [("docs".to_owned(), 1), ("a.txt".to_owned(), 3)]
    );
}

#[test]
fn a_refilled_row_keeps_its_place_and_shows_what_it_became() {
    let ids = Ids::default();
    let mut column = column(&ids, &["docs/", "a.txt"]);
    let mut locked = entry("docs/");
    locked.listable = MetadataValue::Known(false);
    let mut link = entry("a.txt");
    link.kind = Kind::FileSymbolicLink;

    column.apply(Edit::Refill(vec![
        (0, Row::of(&locked, &ids)),
        (1, Row::of(&link, &ids)),
        (7, Row::of(&entry("gone.txt"), &ids)),
    ]));

    assert_eq!(shown(&column), ["docs", "a.txt"]);
    assert_eq!(
        column.entry(1).kind,
        EntryKind::Symlink(strata_components::contract::LinkTarget::File)
    );
    let described = |index: usize| column.rows[column.position_at(index).expect("shown")].described;
    assert_eq!(described(0), super::Described::LockedFolder);
    assert_eq!(described(1), super::Described::FileLink);
}

/// The preferences a grouped folder is shown under: sorted by name, folders first.
fn by_name() -> ViewPreferences {
    ViewPreferences {
        folders_first: true,
        ..ViewPreferences::default()
    }
}

/// Grouped, the rows show in strata-fs's group order (`compare_type_groups` over each type's
/// description), each group in the browser's order; positions and ids still cross as the
/// browser's.
#[test]
fn a_grouped_listing_shows_type_groups_in_strata_fs_order_and_each_in_the_browsers() {
    let ids = Ids::default();
    let names = ["b.txt", "a.png", "docs/", "c.txt", "z.png"];
    let mut column = column(&ids, &names);
    let preferences = by_name();

    assert!(column.set_grouping(Some(preferences)));

    let described = |name: &str| entry_type(&entry(name)).description().to_owned();
    let (text, image) = (described("b.txt"), described("a.png"));
    let mut files = [(text, ["b.txt", "c.txt"]), (image, ["a.png", "z.png"])];
    files.sort_by(|(left, _), (right, _)| compare_type_groups(left, right, preferences));
    let mut expected = vec!["docs".to_owned()];
    expected.extend(files.iter().flat_map(|(_, rows)| rows.map(str::to_owned)));
    assert_eq!(shown(&column), expected);
    for (index, name) in expected.iter().enumerate() {
        let id = column.id_at(index);
        assert_eq!(column.index_of(id), Some(index), "{name}");
        let position = column.position_at(index).expect("a position");
        let browser = names.iter().position(|n| n.trim_end_matches('/') == name);
        assert_eq!(Some(position), browser, "{name}");
    }
}

/// Ungrouped, nothing changes from the browser's order and no lookup is built; turning
/// grouping off again goes back to both.
#[test]
fn an_ungrouped_listing_keeps_the_browsers_order_and_builds_no_lookup() {
    let ids = Ids::default();
    let names = ["b.txt", "a.png", "docs/", "c.txt"];
    let mut column = column(&ids, &names);
    let browser_order = shown(&column);

    assert!(!column.set_grouping(None));
    assert!(column.back.is_empty());
    assert!(column.set_grouping(Some(by_name())));
    assert!(column.set_grouping(None));

    assert_eq!(shown(&column), browser_order);
    assert!(column.back.is_empty());
    let id = column.id_at(2);
    assert_eq!(column.index_of(id), Some(2));
}
