// SPDX-License-Identifier: MIT

use strata_components::{
    collection::{element::CollectionState, selection::Position},
    contract::EntrySource,
};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue, ViewPreferences};

use super::{Applied, Pane};
use crate::{
    events::{Added, Change, Marks},
    listing::{Edit, Load, Row},
    slots::Ids,
};

fn location(depth: usize) -> Location {
    let folders = ["/root", "/root/a", "/root/a/b"];
    Location::local(folders[depth])
}

fn entry(folder: &Location, name: &str) -> FileEntry {
    let path = folder.native_path().expect("a local folder").join(name);
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind: EntryKind::Directory,
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

struct Fixture {
    pane: Pane,
    ids: Ids,
    tx: rune::Tx,
    /// What the browser groups every folder under, or nothing: none of these tests groups.
    grouping: Option<ViewPreferences>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            pane: Pane::new("pane", CollectionState::new()),
            ids: Ids::default(),
            tx: rune::Tx::default(),
            grouping: None,
        }
    }

    fn apply(&mut self, change: Change) -> Applied {
        self.pane.apply(change, &mut self.tx)
    }

    /// The browser opens column `depth`, active or not, and lists one entry in it.
    fn open(&mut self, depth: usize, active: bool) {
        let folder = location(depth);
        let row = Row::of(&entry(&folder, &format!("in-{depth}")), &self.ids);
        self.list((depth, active), &folder, vec![row]);
    }

    fn list(
        &mut self,
        (depth, active): (usize, bool),
        folder: &Location,
        rows: Vec<Row>,
    ) -> Applied {
        let applied = self.apply(Change::Added(Added {
            depth,
            location: folder.clone(),
            show_hidden: false,
            active,
            grouping: self.grouping,
            reveal: false,
        }));
        self.apply(Change::Rows {
            depth,
            edit: Edit::Replace(rows),
            marks: Some(Marks::default()),
            grouping: self.grouping,
        });
        self.apply(Change::Loaded {
            depth,
            load: Load::Listed,
            marks: Marks::default(),
            grouping: self.grouping,
        });
        applied
    }

    /// The folder shown, and its entries.
    fn shows(&self) -> Option<(String, Vec<String>)> {
        let listing = self.pane.listing()?;
        let names = (0..listing.len())
            .map(|index| listing.entry(index).name.to_owned())
            .collect();
        Some((listing.title().to_owned(), names))
    }
}

fn shown(depth: usize) -> Option<(String, Vec<String>)> {
    Some((location(depth).display_name(), vec![format!("in-{depth}")]))
}

#[test]
fn a_column_opened_with_its_parent_kept_active_leaves_the_parent_shown() {
    let mut fixture = Fixture::new();
    fixture.open(0, true);
    fixture.open(1, false);

    assert_eq!(fixture.shows(), shown(0));
    assert_eq!(fixture.pane.depth(), Some(0));
}

#[test]
fn closing_the_shown_column_shows_the_one_the_browser_leaves_active() {
    let mut fixture = Fixture::new();
    fixture.open(0, true);
    fixture.open(1, true);
    fixture.open(2, true);
    assert_eq!(fixture.shows(), shown(2));

    let applied = fixture.apply(Change::Truncated {
        len: 2,
        replacing: false,
        active: Some(1),
    });

    assert_eq!(applied, Applied::Entered);
    assert_eq!(fixture.shows(), shown(1));
}

#[test]
fn after_a_column_closes_the_browsers_cursor_is_adopted_in_the_column_shown() {
    let mut fixture = Fixture::new();
    fixture.open(0, true);
    fixture.open(1, true);
    fixture.open(2, true);
    fixture.apply(Change::Truncated {
        len: 2,
        replacing: false,
        active: Some(1),
    });
    let left = fixture.ids.of(&entry(&location(1), "in-1").location);

    fixture.apply(Change::Selection {
        depth: 1,
        marks: Marks {
            cursor: Some(left),
            selected: vec![left],
            ..Marks::default()
        },
        active: Some(1),
        reveal: true,
        landed: None,
    });

    let selection = &fixture.pane.collection().selection;
    assert_eq!(selection.cursor(), Some(left));
    assert_eq!(selection.len(), 1);
}

/// A column that is not shown keeps up with its rows, and shows them when it shows.
#[test]
fn rows_a_hidden_column_received_show_when_it_shows() {
    let mut fixture = Fixture::new();
    fixture.open(0, true);
    fixture.open(1, true);
    let folder = location(0);
    let row = Row::of(&entry(&folder, "late"), &fixture.ids);
    fixture.apply(Change::Rows {
        depth: 0,
        edit: Edit::Insert(vec![(1, vec![row])]),
        marks: Some(Marks::default()),
        grouping: None,
    });

    fixture.apply(Change::Truncated {
        len: 1,
        replacing: false,
        active: Some(0),
    });

    let names = vec!["in-0".to_owned(), "late".to_owned()];
    assert_eq!(fixture.shows(), Some((folder.display_name(), names)));
}

#[test]
fn a_reset_shows_nothing_until_the_new_root_opens() {
    let mut fixture = Fixture::new();
    fixture.open(0, true);

    assert_eq!(fixture.apply(Change::Reset), Applied::Entered);
    assert_eq!(fixture.shows(), None);
    assert_eq!(fixture.pane.source().len(), 0);
}

fn file(folder: &Location, name: &str) -> FileEntry {
    FileEntry {
        kind: EntryKind::File,
        ..entry(folder, name)
    }
}

#[test]
fn a_grouped_folder_shows_and_steps_in_its_groups_and_an_ungrouped_one_does_not() {
    let mut fixture = Fixture::new();
    fixture.grouping = Some(ViewPreferences::default());
    let folder = location(0);
    fixture.apply(Change::Added(Added {
        depth: 0,
        location: folder.clone(),
        show_hidden: false,
        active: true,
        grouping: fixture.grouping,
        reveal: false,
    }));
    let names = ["b.txt", "a.png", "c.txt"];
    let rows = names
        .iter()
        .map(|name| Row::of(&file(&folder, name), &fixture.ids))
        .collect();
    fixture.apply(Change::Rows {
        depth: 0,
        edit: Edit::Replace(rows),
        marks: Some(Marks::default()),
        grouping: fixture.grouping,
    });
    let id = |name: &str| {
        fixture
            .ids
            .of(&folder.child(std::ffi::OsStr::new(name)).expect("a child"))
    };
    let (b, a, c) = (id("b.txt"), id("a.png"), id("c.txt"));

    assert_eq!(fixture.pane.collection().groups().groups().len(), 2);
    assert_eq!(
        fixture.pane.neighbour(0, b, 1),
        Some(c),
        "b.txt and c.txt are one group"
    );
    assert_eq!(
        fixture.pane.position(0, a),
        Some(1),
        "positions stay the browser's"
    );

    fixture.grouping = None;
    fixture.apply(Change::Loaded {
        depth: 0,
        load: Load::Listed,
        marks: Marks::default(),
        grouping: None,
    });

    assert!(fixture.pane.collection().groups().is_empty());
    assert_eq!(fixture.pane.neighbour(0, b, 1), Some(a));
}

impl Fixture {
    /// The browser opens `folder` in place and lists `names` in it.
    fn visit(&mut self, folder: &Location, names: &[&str]) -> Applied {
        let rows = names
            .iter()
            .map(|name| Row::of(&file(folder, name), &self.ids))
            .collect();
        self.list((0, true), folder, rows)
    }
}

#[test]
fn a_folder_shown_again_comes_back_at_its_position_by_id() {
    let mut fixture = Fixture::new();
    fixture.grouping = Some(ViewPreferences::default());
    let folder = location(0);
    fixture.visit(&folder, &["b.txt", "a.png", "c.txt"]);
    let c = fixture.ids.of(&folder
        .child(std::ffi::OsStr::new("c.txt"))
        .expect("a child"));
    fixture.pane.select_only(0, c);

    let other = Location::local("/elsewhere");
    assert_eq!(
        fixture.visit(&other, &["x.txt"]),
        Applied::Entered,
        "a first visit"
    );
    let back = fixture.visit(&folder, &["c.txt", "b.txt", "a.png"]);

    assert_eq!(back, Applied::Returned);
    let collection = fixture.pane.collection();
    assert_eq!(collection.selection.cursor(), Some(c));
    assert_eq!(
        collection.selection.selected_ids(fixture.pane.source()),
        [c]
    );
}

#[test]
fn a_reveal_wins_over_the_remembered_position() {
    let mut fixture = Fixture::new();
    let folder = location(0);
    fixture.visit(&folder, &["a.txt", "b.txt"]);
    let b = fixture.ids.of(&folder
        .child(std::ffi::OsStr::new("b.txt"))
        .expect("a child"));
    fixture.pane.select_only(0, b);
    fixture.visit(&Location::local("/elsewhere"), &["x.txt"]);

    let revealed = fixture.apply(Change::Added(Added {
        depth: 0,
        location: folder.clone(),
        show_hidden: false,
        active: true,
        grouping: None,
        reveal: true,
    }));
    assert_eq!(revealed, Applied::Entered);
    assert_eq!(fixture.pane.collection().selection.cursor(), None);

    fixture.visit(&Location::local("/elsewhere"), &["x.txt"]);
    assert_eq!(
        fixture.visit(&folder, &["a.txt", "b.txt"]),
        Applied::Returned
    );
}

#[test]
fn a_folder_that_failed_to_read_is_not_remembered() {
    let mut fixture = Fixture::new();
    let folder = location(0);
    fixture.apply(Change::Added(Added {
        depth: 0,
        location: folder.clone(),
        show_hidden: false,
        active: true,
        grouping: None,
        reveal: false,
    }));
    fixture.apply(Change::Loaded {
        depth: 0,
        load: Load::Failed("unreadable".to_owned()),
        marks: Marks::default(),
        grouping: None,
    });
    fixture.visit(&Location::local("/elsewhere"), &["x.txt"]);

    assert_eq!(fixture.visit(&folder, &["a.txt"]), Applied::Entered);
}

#[test]
fn a_restored_cursor_whose_entry_went_falls_back_to_the_selection() {
    let mut fixture = Fixture::new();
    let folder = location(0);
    fixture.visit(&folder, &["a.txt", "b.txt", "c.txt"]);
    let id = |fixture: &Fixture, name: &str| {
        fixture
            .ids
            .of(&folder.child(std::ffi::OsStr::new(name)).expect("a child"))
    };
    let (b, c) = (id(&fixture, "b.txt"), id(&fixture, "c.txt"));
    let (source, collection) = fixture.pane.parts_mut();
    let _ = collection.selection.restore(&Position {
        cursor: Some(c),
        anchor: Some(b),
        selected: vec![b, c],
        ..Position::default()
    });
    collection.note_selection(source);
    fixture.visit(&Location::local("/elsewhere"), &["x.txt"]);

    assert_eq!(
        fixture.visit(&folder, &["a.txt", "b.txt"]),
        Applied::Returned
    );
    let selection = &fixture.pane.collection().selection;
    assert_eq!(selection.cursor(), Some(b), "the first selected entry left");
    assert_eq!(selection.selected_ids(fixture.pane.source()), [b]);
    assert_eq!(selection.anchor(), Some(b));
}

#[test]
fn a_restored_cursor_whose_entry_went_alone_falls_back_to_the_browser_s_cursor() {
    let mut fixture = Fixture::new();
    let folder = location(0);
    fixture.visit(&folder, &["a.txt", "b.txt", "c.txt", "d.txt"]);
    let id = |fixture: &Fixture, name: &str| {
        fixture
            .ids
            .of(&folder.child(std::ffi::OsStr::new(name)).expect("a child"))
    };
    let (a, c) = (id(&fixture, "a.txt"), id(&fixture, "c.txt"));
    fixture.pane.select_only(0, c);
    fixture.visit(&Location::local("/elsewhere"), &["x.txt"]);

    let load = || Marks {
        cursor: Some(a),
        selected: vec![a],
        anchor: Some(a),
        provisional: true,
    };
    fixture.apply(Change::Added(Added {
        depth: 0,
        location: folder.clone(),
        show_hidden: false,
        active: true,
        grouping: None,
        reveal: false,
    }));
    let rows = ["a.txt", "b.txt", "d.txt"]
        .iter()
        .map(|name| Row::of(&file(&folder, name), &fixture.ids))
        .collect();
    fixture.apply(Change::Rows {
        depth: 0,
        edit: Edit::Replace(rows),
        marks: Some(load()),
        grouping: None,
    });
    fixture.apply(Change::Loaded {
        depth: 0,
        load: Load::Listed,
        marks: load(),
        grouping: None,
    });

    assert_eq!(fixture.pane.collection().selection.cursor(), Some(a));
}

#[test]
fn a_restored_cursor_whose_entry_went_with_no_browser_cursor_falls_back_to_the_first_row() {
    let mut fixture = Fixture::new();
    let folder = location(0);
    fixture.visit(&folder, &["a.txt", "b.txt", "c.txt", "d.txt"]);
    let id = |fixture: &Fixture, name: &str| {
        fixture
            .ids
            .of(&folder.child(std::ffi::OsStr::new(name)).expect("a child"))
    };
    let (a, c) = (id(&fixture, "a.txt"), id(&fixture, "c.txt"));
    fixture.pane.select_only(0, c);
    fixture.visit(&Location::local("/elsewhere"), &["x.txt"]);

    assert_eq!(
        fixture.visit(&folder, &["a.txt", "b.txt", "d.txt"]),
        Applied::Returned
    );
    let selection = &fixture.pane.collection().selection;
    assert_eq!(selection.cursor(), Some(a));
    assert!(selection.selected_ids(fixture.pane.source()).is_empty());
}
