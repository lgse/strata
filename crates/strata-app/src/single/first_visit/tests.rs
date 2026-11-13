// SPDX-License-Identifier: MIT

//! When a first visit gives the first row the cursor, on a pane the browser's changes are
//! scripted into: a hosted test cannot hold a load open between its rows and its end.

use strata_components::{
    collection::{element::CollectionState, selection::AnchorRule},
    contract::EntryId,
};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::FirstVisit;
use crate::{
    events::{Added, Change, Marks},
    listing::{Edit, Load, Row},
    pane::Pane,
    slots::Ids,
};

const NAMES: [&str; 3] = ["a", "b", "c"];

fn folder() -> Location {
    Location::local("/root")
}

fn entry(name: &str) -> FileEntry {
    let path = folder().native_path().expect("a local folder").join(name);
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
        visit.pane.apply(
            Change::Added(Added {
                depth: 0,
                location: folder(),
                show_hidden: false,
                active: true,
                grouping: None,
            }),
            &mut visit.tx,
        );
        visit.visit.arm();
        visit
    }

    fn id(&self, name: &str) -> EntryId {
        self.ids.of(&entry(name).location)
    }

    fn marks(&self, cursor: Option<&str>) -> Marks {
        let cursor = cursor.map(|name| self.id(name));
        Marks {
            cursor,
            selected: cursor.into_iter().collect(),
            provisional: false,
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
        };
        self.pane.apply(change, &mut self.tx);
    }

    /// A poll after each step, as the view polls between the browser's dispatches.
    fn run(mut self, steps: &[Step]) -> Option<String> {
        for step in steps {
            self.step(*step);
            let _ = self.visit.settle(&mut self.pane);
        }
        let cursor = self.pane.collection().selection.cursor()?;
        NAMES
            .into_iter()
            .find(|name| self.id(name) == cursor)
            .map(str::to_owned)
    }
}

#[test]
fn the_first_row_takes_the_cursor_only_when_nothing_else_placed_one_by_the_loads_end() {
    use Step::{Failed, Loaded, Moves, Rows, Touches};
    let cases: [(&str, &[Step], Option<&str>); 7] = [
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
    ];
    for (case, steps, expected) in cases {
        assert_eq!(Visit::opened().run(steps).as_deref(), expected, "{case}");
    }
}
