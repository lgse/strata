// SPDX-License-Identifier: MIT

use std::{cell::RefCell, collections::HashSet, path::Path};

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::{Details, Filler, Fills, SCROLL_SETTLE, ScrollSettle};
use crate::{
    listing::{Column, Edit, Row},
    slots::Ids,
};

const FOLDER: &str = "/photos";

fn photo(index: usize) -> FileEntry {
    let name = format!("photo-{index}.png");
    FileEntry {
        location: Location::local(Path::new(FOLDER).join(&name)),
        thumbnail_path: None,
        native_name: name.clone().into(),
        display_name: name,
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
        listable: MetadataValue::Unavailable,
        is_hidden: false,
    }
}

/// A browser that records what it is asked: each fill by position, and each priority call.
#[derive(Default)]
struct Recorder {
    entries: RefCell<Vec<FileEntry>>,
    requests: RefCell<Vec<usize>>,
    /// Each fill's position with whether it was asked with details.
    details: RefCell<Vec<(usize, bool)>>,
    priorities: RefCell<usize>,
}

impl Recorder {
    fn of(len: usize) -> Self {
        Self {
            entries: RefCell::new((0..len).map(photo).collect()),
            ..Self::default()
        }
    }

    fn take(&self) -> Vec<usize> {
        std::mem::take(&mut self.requests.borrow_mut())
    }
}

impl Filler for Recorder {
    fn entry_at(&self, _depth: usize, position: usize) -> Option<FileEntry> {
        self.entries.borrow().get(position).cloned()
    }

    fn request(&self, (_, position): (usize, usize), _location: Location, details: bool) {
        self.requests.borrow_mut().push(position);
        self.details.borrow_mut().push((position, details));
    }

    fn prioritize(&self, _depth: usize, _near: &[Location]) {
        *self.priorities.borrow_mut() += 1;
    }

    fn touched(&self, _location: &Location) -> bool {
        false
    }
}

fn listing(browser: &Recorder, ids: &Ids) -> Column {
    let rows = browser
        .entries
        .borrow()
        .iter()
        .map(|entry| Row::of(entry, ids))
        .collect();
    let mut column = Column::new(Location::local(FOLDER), false, None);
    column.apply(Edit::Replace(rows));
    column
}

/// A scroll through a long folder: however often the range changes between two polls, a
/// poll asks once, for the rows of the last range and a quarter of it each way. A row is
/// asked for at most once while its fill is out, and a row never near the viewport at a
/// poll is never asked for.
#[test]
fn a_scroll_asks_each_near_row_once_per_poll() {
    let browser = Recorder::of(1_000);
    let ids = Ids::default();
    let column = listing(&browser, &ids);
    let mut fills = Fills::new(Details::All);

    fills.see(0..20);
    fills.request(&browser, 0, &column);
    assert_eq!(
        browser.take(),
        (0..25).collect::<Vec<_>>(),
        "on screen and the margin"
    );
    assert_eq!(*browser.priorities.borrow(), 1);

    fills.request(&browser, 0, &column);
    assert!(browser.take().is_empty(), "nothing changed");
    assert_eq!(*browser.priorities.borrow(), 1);

    for start in 1..=40 {
        fills.see(start..start + 20);
    }
    fills.request(&browser, 0, &column);
    assert_eq!(*browser.priorities.borrow(), 2, "one poll, one batch");
    assert_eq!(
        browser.take(),
        (35..65).collect::<Vec<_>>(),
        "only the last range and its margin: 25..35 was never near at a poll"
    );

    let mut asked = HashSet::new();
    for start in (40..=500).step_by(7) {
        fills.see(start..start + 20);
        fills.request(&browser, 0, &column);
        for position in browser.take() {
            assert!(asked.insert(position), "row {position} asked for twice");
        }
    }
    let last = 500 + 20 + 5;
    assert!(asked.iter().all(|position| (65..last).contains(position)));
    assert!(!asked.contains(&30), "a row passed between two polls");
    assert!(!asked.contains(&last), "the row past the margin");
}

/// A row is asked for again only once its fill landed and it still lacks something, and
/// not at all once it lacks nothing; a new folder starts over.
#[test]
fn a_row_is_asked_again_only_after_its_fill_landed() {
    let browser = Recorder::of(4);
    let ids = Ids::default();
    let column = listing(&browser, &ids);
    let mut fills = Fills::new(Details::All);
    fills.see(0..4);
    fills.request(&browser, 0, &column);
    assert_eq!(browser.take(), vec![0, 1, 2, 3]);

    {
        let mut entries = browser.entries.borrow_mut();
        // The basic fill landed; the image's dimensions are still to come.
        entries[0].size = MetadataValue::Known(10);
        entries[0].modified_unix_seconds = MetadataValue::Known(1);
        entries[0].mode = MetadataValue::Known(0o644);
        // Everything landed.
        let filled = entries[0].clone();
        let row = &mut entries[1];
        row.size = filled.size;
        row.modified_unix_seconds = filled.modified_unix_seconds;
        row.mode = filled.mode;
        row.image_dimensions = MetadataValue::Known((320, 180));
    }
    let mut column = column;
    let refilled = (0..2)
        .map(|position| (position, Row::of(&browser.entries.borrow()[position], &ids)))
        .collect();
    column.apply(Edit::Refill(refilled));
    fills.request(&browser, 0, &column);
    assert_eq!(
        browser.take(),
        vec![0],
        "only the row that changed and still lacks"
    );

    fills.see(0..3);
    fills.request(&browser, 0, &column);
    assert!(browser.take().is_empty(), "every row still out or filled");

    fills.request(&browser, 1, &column);
    assert_eq!(browser.take(), vec![0, 2, 3], "another folder starts over");
}

/// A row that left the range and came back is asked for again, its fill never having
/// landed, as GTK asks again for an item it binds again.
#[test]
fn a_row_scrolled_out_and_back_is_asked_again() {
    let browser = Recorder::of(200);
    let ids = Ids::default();
    let column = listing(&browser, &ids);
    let mut fills = Fills::new(Details::All);
    fills.see(0..20);
    fills.request(&browser, 0, &column);
    assert!(browser.take().contains(&3));

    fills.see(100..120);
    fills.request(&browser, 0, &column);
    assert!(!browser.take().contains(&3));

    fills.see(0..20);
    fills.request(&browser, 0, &column);
    assert_eq!(
        browser.take(),
        (0..25).collect::<Vec<_>>(),
        "every row back in the range"
    );
}

/// GTK's List asks a folder's fill with details, for its item count, and a file's without
/// (`list_factory.rs`, `details = entry.is_directory()`); so a file that lacks only an
/// image's dimensions is not asked for at all, and a folder that lacks only its count is.
/// The Icons view asks every row with details.
#[test]
fn the_list_view_asks_a_folders_count_and_no_files_details() {
    let known = |mut entry: FileEntry| {
        entry.size = MetadataValue::Known(1);
        entry.modified_unix_seconds = MetadataValue::Known(1);
        entry.mode = MetadataValue::Known(0o644);
        entry
    };
    let mut folder = known(photo(0));
    folder.kind = EntryKind::Directory;
    folder.listable = MetadataValue::Known(true);
    let unread = photo(1);
    let image = known(photo(2));
    let browser = Recorder {
        entries: RefCell::new(vec![folder, unread, image]),
        ..Recorder::default()
    };
    let ids = Ids::default();
    let column = listing(&browser, &ids);
    for (details, expected) in [
        (Details::Folders, vec![(0, true), (1, false)]),
        (Details::All, vec![(0, true), (1, true), (2, true)]),
    ] {
        let mut fills = Fills::new(details);
        fills.see(0..3);
        fills.request(&browser, 0, &column);
        assert_eq!(
            std::mem::take(&mut *browser.details.borrow_mut()),
            expected,
            "{details:?}"
        );
    }
}

/// GTK's Columns rows register a fill only when the time, a file's size or a folder's
/// `listable` is unknown, or for a folder's count while sizes show (`bind.rs:254-257`): a row
/// that lacks only its mode is asked by Columns only for a folder while sizes show, and by
/// the List view always.
#[test]
fn a_columns_row_lacking_only_its_mode_is_asked_only_for_a_folders_size() {
    let only_mode = |mut entry: FileEntry| {
        entry.size = MetadataValue::Known(1);
        entry.modified_unix_seconds = MetadataValue::Known(1);
        entry.child_count = MetadataValue::Known(1);
        entry
    };
    let mut folder = only_mode(photo(0));
    folder.kind = EntryKind::Directory;
    folder.listable = MetadataValue::Known(true);
    let file = only_mode(photo(1));
    let browser = Recorder {
        entries: RefCell::new(vec![folder, file]),
        ..Recorder::default()
    };
    let ids = Ids::default();
    let column = listing(&browser, &ids);
    for (how, mut fills, expected) in [
        (
            "Columns, sizes shown",
            Fills::unfilled(Details::Folders),
            vec![(0, true)],
        ),
        (
            "Columns, sizes hidden",
            Fills::unfilled(Details::Off),
            vec![],
        ),
        (
            "List",
            Fills::new(Details::Folders),
            vec![(0, true), (1, false)],
        ),
    ] {
        fills.see(0..2);
        fills.request(&browser, 0, &column);
        assert_eq!(
            std::mem::take(&mut *browser.details.borrow_mut()),
            expected,
            "{how}"
        );
    }
}

#[test]
fn scroll_settle_defers_during_thumb_drag_and_flushes_immediately_on_release() {
    let mut settle = ScrollSettle::default();
    let folder = Location::local("/tmp/big");
    settle.visit(&folder);

    // Initial range is ready immediately.
    settle.see(&(0..30));
    assert_eq!(settle.settled(0.0, false), Ok(()));
    assert_eq!(settle.settled_for_drag(0.0, false), Ok(()));

    // Normal wheel scroll defers metadata fills until SCROLL_SETTLE, but not thumbnails.
    settle.see(&(10..40));
    assert!(settle.settled(1.0, false).is_err());
    assert_eq!(settle.settled_for_drag(1.0, false), Ok(()));

    // Active thumb drag defers both fills and thumbnails while moving.
    settle.see(&(500..530));
    assert!(settle.settled(2.0, true).is_err());
    assert!(settle.settled_for_drag(2.0, true).is_err());

    // Pausing while still holding the thumb flushes after SCROLL_SETTLE.
    assert!(settle.settled(2.0 + SCROLL_SETTLE / 2.0, true).is_err());
    assert_eq!(settle.settled(2.0 + SCROLL_SETTLE + 0.001, true), Ok(()));

    // Moving again mid-drag re-arms deferral, then releasing flushes immediately with 0ms wait.
    settle.see(&(9000..9030));
    assert!(settle.settled(3.0, true).is_err());
    assert!(settle.settled_for_drag(3.0, true).is_err());
    assert_eq!(
        settle.settled(3.016, false),
        Ok(()),
        "releasing thumb flushes fills immediately"
    );
    assert_eq!(
        settle.settled_for_drag(3.016, false),
        Ok(()),
        "releasing thumb flushes thumbnails immediately"
    );
}

/// When a reload cancels in-flight metadata fills and replaces the same folder with fresh
/// `Unknown` entries, `invalidate()` clears `asked` so the next poll re-requests them.
#[test]
fn invalidate_readmits_rows_after_reload_cancels_in_flight_fills() {
    let browser = Recorder::of(4);
    let ids = Ids::default();
    let mut column = listing(&browser, &ids);
    let mut fills = Fills::new(Details::All);

    fills.see(0..4);
    fills.request(&browser, 0, &column);
    assert_eq!(browser.take(), vec![0, 1, 2, 3]);

    // Simulate a reload of the same folder replacing entries with fresh Unknown rows after
    // cancelling the previous metadata load.
    let reloaded_rows = browser
        .entries
        .borrow()
        .iter()
        .map(|entry| Row::of(entry, &ids))
        .collect();
    column.apply(Edit::Replace(reloaded_rows));

    fills.invalidate();
    fills.request(&browser, 0, &column);
    assert_eq!(
        browser.take(),
        vec![0, 1, 2, 3],
        "reloaded rows must be re-requested after invalidate()"
    );
}
