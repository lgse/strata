// SPDX-License-Identifier: MIT

//! When a first visit gives the first row the cursor, on a pane the browser's changes are
//! scripted into: a hosted test cannot hold a load open between its rows and its end.

use strata_components::{
    collection::{element::CollectionState, selection::AnchorRule},
    contract::{EntryId, EntrySource},
};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::FirstVisit;
use crate::{
    events::{Added, Change, Marks, Moved},
    listing::{Edit, Load, Row},
    pane::Pane,
    slots::Ids,
};

const NAMES: [&str; 3] = ["a", "b", "c"];

fn folder() -> Location {
    Location::local("/root")
}

/// Where the folder is after it was renamed elsewhere.
fn renamed() -> Location {
    Location::local("/renamed")
}

fn entry(name: &str) -> FileEntry {
    entry_in(&folder(), name)
}

fn entry_in(folder: &Location, name: &str) -> FileEntry {
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

/// What reaches the pane between the browser's events, and the pane's polls.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// The folder's rows arrive, the folder still loading.
    Rows(&'static [&'static str]),
    /// The load ends, the browser's cursor on this entry, or none.
    Loaded(Option<&'static str>),
    /// The read fails.
    Failed,
    /// The user moves the cursor to this entry, as a Down or a click does.
    Moves(&'static str),
    /// The user's input reaches the pane and changes nothing, as a scroll does.
    Touches,
    /// The folder shown is renamed elsewhere, its rows read again under the new name.
    Relocated,
    /// A child column opens, is read, and takes the browser's cursor on its one entry.
    Child,
    /// The child column closes, and the folder shows again, already listed.
    Closed,
}

struct Visit {
    pane: Pane,
    visit: FirstVisit,
    ids: Ids,
    tx: rune::Tx,
}

impl Visit {
    /// Another folder shows, as a navigation shows it.
    fn opened() -> Self {
        let mut visit = Self {
            pane: Pane::new("pane", CollectionState::new()),
            visit: FirstVisit::default(),
            ids: Ids::default(),
            tx: rune::Tx::default(),
        };
        visit.apply(Change::Added(Added {
            depth: 0,
            location: folder(),
            show_hidden: false,
            active: true,
            grouping: None,
            reveal: false,
        }));
        visit
    }

    /// A change the view's listener hands the pane, which the visit follows.
    fn apply(&mut self, change: Change) {
        let applied = self.pane.apply(change, &mut self.tx);
        self.visit.follow(applied, &self.pane);
    }

    fn id(&self, name: &str) -> EntryId {
        self.ids.of(&entry(name).location)
    }

    fn marks(&self, cursor: Option<&str>) -> Marks {
        let cursor = cursor.map(|name| self.id(name));
        Marks {
            cursor,
            selected: cursor.into_iter().collect(),
            ..Marks::default()
        }
    }

    fn step(&mut self, step: Step) {
        let change = match step {
            Step::Rows(names) => Change::Rows {
                depth: 0,
                edit: Edit::Replace(
                    names
                        .iter()
                        .map(|name| Row::of(&entry(name), &self.ids))
                        .collect(),
                ),
                marks: Some(Marks::default()),
                grouping: None,
            },
            Step::Loaded(cursor) => Change::Loaded {
                depth: 0,
                load: Load::Listed,
                marks: self.marks(cursor),
                grouping: None,
            },
            Step::Failed => Change::Loaded {
                depth: 0,
                load: Load::Failed("unreadable".to_owned()),
                marks: Marks::default(),
                grouping: None,
            },
            Step::Moves(name) => {
                let id = self.id(name);
                self.visit.interrupt();
                let (source, collection) = self.pane.parts_mut();
                let index = (0..source.len())
                    .find(|index| source.id_at(*index) == id)
                    .expect("a row shown");
                let _ = collection
                    .selection
                    .move_cursor(source, index, AnchorRule::Move);
                return;
            }
            Step::Touches => {
                self.visit.interrupt();
                return;
            }
            Step::Child => {
                let child = Location::local("/root/inner");
                let row = Row::of(&entry_in(&child, "deep"), &self.ids);
                let cursor = Some(self.ids.of(&entry_in(&child, "deep").location));
                for change in [
                    Change::Added(Added {
                        depth: 1,
                        location: child.clone(),
                        show_hidden: false,
                        active: true,
                        grouping: None,
                        reveal: false,
                    }),
                    Change::Rows {
                        depth: 1,
                        edit: Edit::Replace(vec![row]),
                        marks: Some(Marks::default()),
                        grouping: None,
                    },
                    Change::Loaded {
                        depth: 1,
                        load: Load::Listed,
                        marks: Marks {
                            cursor,
                            selected: cursor.into_iter().collect(),
                            ..Marks::default()
                        },
                        grouping: None,
                    },
                ] {
                    self.apply(change);
                    let _ = self.visit.settle(&mut self.pane);
                }
                return;
            }
            Step::Closed => Change::Truncated {
                len: 1,
                replacing: false,
                active: Some(0),
            },
            Step::Relocated => Change::Relocated {
                from_depth: 0,
                columns: vec![Moved {
                    location: renamed(),
                    rows: NAMES
                        .iter()
                        .map(|name| Row::of(&entry_in(&renamed(), name), &self.ids))
                        .collect(),
                    marks: Marks::default(),
                    grouping: None,
                }],
            },
        };
        self.apply(change);
    }

    /// A poll after each step, as the view polls between the browser's dispatches.
    fn run(mut self, steps: &[Step]) -> Option<String> {
        for step in steps {
            self.step(*step);
            let _ = self.visit.settle(&mut self.pane);
        }
        let cursor = self.pane.collection().selection.cursor()?;
        let listing = self.pane.listing()?;
        let index = listing.index_of(cursor)?;
        Some(listing.entry(index).name.to_owned())
    }
}

#[test]
fn the_first_row_takes_the_cursor_only_when_nothing_else_placed_one_by_the_loads_end() {
    use Step::{Child, Closed, Failed, Loaded, Moves, Relocated, Rows, Touches};
    let cases: [(&str, &[Step], Option<&str>); 9] = [
        (
            "rows first, then the load's end with no cursor",
            &[Rows(&NAMES), Loaded(None)],
            Some("a"),
        ),
        (
            "rows that came before the load's end take no cursor yet",
            &[Rows(&NAMES)],
            None,
        ),
        (
            "the browser's cursor at the load's end, as Back or Alt+Up place it",
            &[Rows(&NAMES), Loaded(Some("c"))],
            Some("c"),
        ),
        (
            "a Down or a click before the load's end",
            &[Rows(&NAMES), Moves("b"), Loaded(Some("b"))],
            Some("b"),
        ),
        (
            "a scroll before the load's end",
            &[Rows(&NAMES), Touches, Loaded(None)],
            None,
        ),
        (
            "an empty folder that gains a file later",
            &[Loaded(None), Rows(&NAMES[..1])],
            None,
        ),
        (
            "a folder that could not be read",
            &[Failed, Rows(&NAMES)],
            None,
        ),
        (
            "the folder shown renamed elsewhere, which is no navigation",
            &[Rows(&NAMES), Touches, Loaded(None), Relocated],
            None,
        ),
        (
            "a closed column that shows its parent again, already listed",
            &[Rows(&NAMES), Touches, Loaded(None), Child, Closed],
            None,
        ),
    ];
    for (case, steps, expected) in cases {
        assert_eq!(Visit::opened().run(steps).as_deref(), expected, "{case}");
    }
}
