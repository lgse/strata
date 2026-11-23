// SPDX-License-Identifier: MIT

use std::{path::Path, rc::Rc};

use rune::{Key, Mods};

use strata_components::collection::{
    element::CollectionState,
    selection::{Extend, Position},
};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::{adapters::LocalFileSource, app::Browser, app::BrowserEvent};

use super::{Moving, adopt, sync_rule};
use crate::{
    App,
    events::{self, Change, Marks},
    listing::{Column, Edit, Load, Row},
    slots::Ids,
    test_support::{
        PATIENCE, fixture,
        view::{Shape, browser_holds, hosted_in, idle, open, press, wait},
    },
};

const MODULE: &str = "handover::tests";

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

const LIST: Shape = Shape {
    described: "List view",
    list: "list",
    entry: "list-item",
};

fn file_entry(path: &Path) -> FileEntry {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_owned();
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.clone().into(),
        display_name: name,
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Known(0o644),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn moving_consumes_arrivals_once_and_skips_already_held_targets() {
    let ids = Ids::default();
    let root = Location::local(Path::new("/tmp/root"));
    let old_entry = file_entry(Path::new("/tmp/root/old.txt"));
    let new_entry = file_entry(Path::new("/tmp/root/new.txt"));
    let from = ids.of(&old_entry.location);
    let to = ids.of(&new_entry.location);

    let mut column = Column::new(root, false, None);
    column.apply(Edit::Replace(vec![Row::of(&old_entry, &ids)]));
    column.set_load(Load::Listed);
    let mut collection = CollectionState::new();
    let _ = collection.selection.restore(&Position {
        cursor: Some(from),
        cursor_index: Some(0),
        anchor: Some(from),
        selected: vec![from],
        provisional: false,
        scroll: None,
    });

    let mut moving = Moving::default();
    moving.moved(from, to, |id| column.position_of(id).is_some());

    column.apply(Edit::Replace(vec![Row::of(&new_entry, &ids)]));
    assert_eq!(moving.arrived(&column, &mut collection), vec![(from, to)]);
    collection.selection.sync(&column, sync_rule(&column));
    assert!(collection.selection.contains(&column, to));

    // A second edit on the same column must not re-apply the consumed relocation onto `to`.
    assert!(moving.arrived(&column, &mut collection).is_empty());

    // Even if a fresh pair targets an id the selection already holds, `arrived` does not panic.
    let other_from = ids.of(&Location::local(Path::new("/tmp/other/old.txt")));
    moving.moved(other_from, to, |id| id == other_from);
    assert_eq!(
        moving.arrived(&column, &mut collection),
        vec![(other_from, to)]
    );
    assert!(collection.selection.contains(&column, to));
}

#[test]
fn moving_relocates_live_ranges_across_same_length_and_resized_edits() {
    let ids = Ids::default();
    let root = Location::local(Path::new("/tmp/root"));
    let a = file_entry(Path::new("/tmp/root/a.txt"));
    let old = file_entry(Path::new("/tmp/root/old.txt"));
    let c = file_entry(Path::new("/tmp/root/c.txt"));
    let new = file_entry(Path::new("/tmp/root/new.txt"));
    let extra = file_entry(Path::new("/tmp/root/z.txt"));
    let (id_a, id_old, id_c, id_new) = (
        ids.of(&a.location),
        ids.of(&old.location),
        ids.of(&c.location),
        ids.of(&new.location),
    );

    let mut column = Column::new(root.clone(), false, None);
    column.apply(Edit::Replace(vec![
        Row::of(&a, &ids),
        Row::of(&old, &ids),
        Row::of(&c, &ids),
    ]));
    column.set_load(Load::Listed);
    let mut collection = CollectionState::new();
    collection.selection.select_all(&column);

    let mut moving = Moving::default();
    moving.moved(id_old, id_new, |id| column.position_of(id).is_some());
    column.apply(Edit::Replace(vec![
        Row::of(&a, &ids),
        Row::of(&new, &ids),
        Row::of(&c, &ids),
    ]));
    assert_eq!(
        moving.arrived(&column, &mut collection),
        vec![(id_old, id_new)]
    );
    collection.selection.sync(&column, sync_rule(&column));
    assert_eq!(
        collection.selection.selected_ids(&column),
        vec![id_a, id_new, id_c]
    );

    let mut column2 = Column::new(root, false, None);
    column2.apply(Edit::Replace(vec![Row::of(&a, &ids), Row::of(&old, &ids)]));
    column2.set_load(Load::Listed);
    let mut collection2 = CollectionState::new();
    collection2.selection.select_only(&column2, 0);
    collection2
        .selection
        .extend_to(&column2, 1, Extend::Replace);

    let mut moving2 = Moving::default();
    moving2.moved(id_old, id_new, |id| column2.position_of(id).is_some());
    column2.apply(Edit::Replace(vec![
        Row::of(&a, &ids),
        Row::of(&new, &ids),
        Row::of(&extra, &ids),
    ]));
    assert_eq!(
        moving2.arrived(&column2, &mut collection2),
        vec![(id_old, id_new)]
    );
    collection2.selection.sync(&column2, sync_rule(&column2));
    assert_eq!(
        collection2.selection.selected_ids(&column2),
        vec![id_a, id_new]
    );
}

#[test]
fn moving_skips_relocate_when_target_is_held_in_base_and_flipped_off() {
    let ids = Ids::default();
    let root = Location::local(Path::new("/tmp/root"));
    let a = file_entry(Path::new("/tmp/root/a.txt"));
    let c = file_entry(Path::new("/tmp/root/c.txt"));
    let new = file_entry(Path::new("/tmp/root/new.txt"));
    let id_new = ids.of(&new.location);

    let mut column = Column::new(root, false, None);
    column.apply(Edit::Replace(vec![
        Row::of(&a, &ids),
        Row::of(&c, &ids),
        Row::of(&new, &ids),
    ]));
    column.set_load(Load::Listed);
    let mut collection = CollectionState::new();
    collection.selection.select_only(&column, 0);
    collection.selection.toggle(&column, 2);
    collection.selection.extend_to(&column, 1, Extend::Add);
    collection.selection.toggle_in_run(&column, 2);
    assert!(!collection.selection.contains(&column, id_new));
    assert!(collection.selection.holds(id_new));

    let other_from = ids.of(&Location::local(Path::new("/tmp/other/old.txt")));
    let mut moving = Moving::default();
    moving.moved(other_from, id_new, |id| id == other_from);
    assert_eq!(
        moving.arrived(&column, &mut collection),
        vec![(other_from, id_new)]
    );
}

#[test]
fn item_relocated_only_emits_change_moved_within_same_parent_folder() {
    let browser = Browser::new(Rc::new(LocalFileSource));
    let ids = Ids::default();

    let same_folder = BrowserEvent::ItemRelocated {
        from: Location::local(Path::new("/tmp/root/a.txt")),
        to: Location::local(Path::new("/tmp/root/b.txt")),
        merged: false,
    };
    let same_read = events::read(&same_folder, &browser, &ids);
    assert!(
        same_read
            .changes
            .iter()
            .any(|change| matches!(change, Change::Moved { .. }))
    );

    let cross_folder = BrowserEvent::ItemRelocated {
        from: Location::local(Path::new("/tmp/root/a.txt")),
        to: Location::local(Path::new("/tmp/root/dest/a.txt")),
        merged: false,
    };
    let cross_read = events::read(&cross_folder, &browser, &ids);
    assert!(cross_read.changes.is_empty());
}

#[test]
fn adopt_updates_anchor_when_cursor_and_selection_already_match() {
    let ids = Ids::default();
    let root = Location::local(Path::new("/tmp/root"));
    let a = file_entry(Path::new("/tmp/root/a.txt"));
    let b = file_entry(Path::new("/tmp/root/b.txt"));
    let id_a = ids.of(&a.location);
    let id_b = ids.of(&b.location);

    let mut column = Column::new(root, false, None);
    column.apply(Edit::Replace(vec![Row::of(&a, &ids), Row::of(&b, &ids)]));
    column.set_load(Load::Listed);
    let mut collection = CollectionState::new();
    let _ = collection.selection.restore(&Position {
        cursor: Some(id_b),
        cursor_index: Some(1),
        anchor: Some(id_a),
        selected: vec![id_b],
        provisional: false,
        scroll: None,
    });

    adopt(
        &column,
        &mut collection,
        &Marks {
            cursor: Some(id_b),
            selected: vec![id_b],
            anchor: Some(id_b),
            provisional: false,
        },
    );
    assert_eq!(collection.selection.anchor(), Some(id_b));
}

struct Moved {
    mode: &'static str,
    shape: Shape,
    reach: &'static [Key],
    moved: &'static str,
    neighbour: &'static str,
    arrow: Key,
    arrowed: &'static str,
    name: &'static str,
}

fn a_move_out_leaves_the_keyboard_on_the_neighbour(case: Moved) {
    let shape = case.shape;
    hosted_in(MODULE, case.mode, case.name, move |mut host| {
        let folder = fixture();
        let root = folder.path().to_owned();
        open(&mut host, shape, &root);
        for key in case.reach {
            press(&mut host, key.clone(), Mods::default());
        }
        wait(
            &mut host,
            shape,
            "the keyboard on the entry to move",
            |seen| seen.focused.as_deref() == Some(case.moved),
        );
        let (from, into) = (root.join(case.moved), root.join("archive"));
        host.runtime().update(move |app: &mut App, _| {
            let sources = vec![Location::local(&from)];
            crate::transfer::start_drop_transfer(app, Location::local(&into), sources, true);
        });
        let arrived = root.join("archive").join(case.moved);
        assert!(
            host.run_until(PATIENCE, |_| arrived.exists()),
            "the entry moved"
        );
        let seen = wait(&mut host, shape, "the keyboard on the neighbour", |seen| {
            seen.focused.as_deref() == Some(case.neighbour)
        });
        assert!(seen.selected.is_empty(), "nothing selected: {seen:#?}");
        idle(&mut host);
        assert_eq!(
            browser_holds(&host),
            (Vec::new(), None),
            "the browser holds no cursor"
        );

        press(&mut host, case.arrow, Mods::default());
        let seen = wait(&mut host, shape, "the arrow to go on", |seen| {
            seen.focused.as_deref() == Some(case.arrowed)
        });
        assert_eq!(seen.selected, [case.arrowed]);
        let holds = (vec![case.arrowed.to_owned()], Some(case.arrowed.to_owned()));
        assert_eq!(browser_holds(&host), holds, "the browser follows the arrow");
    });
}

fn last_row(mode: &'static str, shape: Shape, arrow: Key, name: &'static str) -> Moved {
    Moved {
        mode,
        shape,
        reach: &[Key::End],
        moved: "todo.txt",
        neighbour: "readme.md",
        arrow,
        arrowed: "pictures",
        name,
    }
}

#[test]
fn a_move_out_leaves_the_keyboard_on_the_neighbour_in_icons() {
    let name = "a_move_out_leaves_the_keyboard_on_the_neighbour_in_icons";
    a_move_out_leaves_the_keyboard_on_the_neighbour(last_row("icons", ICONS, Key::Left, name));
}

#[test]
fn a_move_out_leaves_the_keyboard_on_the_neighbour_in_list() {
    let name = "a_move_out_leaves_the_keyboard_on_the_neighbour_in_list";
    a_move_out_leaves_the_keyboard_on_the_neighbour(last_row("list", LIST, Key::Up, name));
}

#[test]
fn a_move_out_mid_list_leaves_the_keyboard_on_the_row_now_at_its_position() {
    a_move_out_leaves_the_keyboard_on_the_neighbour(Moved {
        mode: "list",
        shape: LIST,
        reach: &[Key::End, Key::Up],
        moved: "readme.md",
        neighbour: "todo.txt",
        arrow: Key::Up,
        arrowed: "pictures",
        name: "a_move_out_mid_list_leaves_the_keyboard_on_the_row_now_at_its_position",
    });
}

#[test]
fn an_undone_move_out_leaves_the_keyboard_on_the_neighbour() {
    let name = "an_undone_move_out_leaves_the_keyboard_on_the_neighbour";
    hosted_in(MODULE, "list", name, move |mut host| {
        let folder = fixture();
        let root = folder.path().to_owned();
        open(&mut host, LIST, &root);
        press(&mut host, Key::End, Mods::default());
        press(&mut host, Key::Up, Mods::default());
        wait(&mut host, LIST, "the keyboard on readme.md", |seen| {
            seen.focused.as_deref() == Some("readme.md")
        });
        let (from, into) = (root.join("readme.md"), root.join("archive"));
        host.runtime().update(move |app: &mut App, _| {
            let sources = vec![Location::local(&from)];
            crate::transfer::start_drop_transfer(app, Location::local(&into), sources, true);
        });
        let moved = root.join("archive/readme.md");
        assert!(
            host.run_until(PATIENCE, |_| moved.exists()),
            "the entry moved"
        );
        wait(&mut host, LIST, "the keyboard on the neighbour", |seen| {
            seen.focused.as_deref() == Some("todo.txt")
        });
        let ctrl = Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        };
        press(&mut host, Key::Command('z'), ctrl);
        let back = root.join("readme.md");
        assert!(
            host.run_until(PATIENCE, |_| back.exists()),
            "undo brought it back"
        );
        wait(&mut host, LIST, "readme.md listed again", |seen| {
            seen.entries.iter().any(|[name, _]| name == "readme.md")
        });
        idle(&mut host);
        let seen = wait(&mut host, LIST, "the view at rest", |_| true);
        assert_eq!(seen.focused.as_deref(), Some("todo.txt"), "{seen:#?}");
        assert!(seen.selected.is_empty(), "nothing selected: {seen:#?}");
    });
}

fn image_entry(path: &Path, modified: MetadataValue<i64>, size: MetadataValue<u64>) -> FileEntry {
    FileEntry {
        modified_unix_seconds: modified,
        size,
        ..file_entry(path)
    }
}

#[test]
fn moving_relocates_thumbnail_across_rename_and_avoids_re_request() {
    use strata_components::{
        contract::EntrySource,
        thumbnails::{Outcome, State, Step, ThumbnailStore, Viewport},
    };

    let ids = Ids::default();
    let root = Location::local(Path::new("/tmp/photos"));
    let old_path = Path::new("/tmp/photos/photo-0.png");
    let new_path = Path::new("/tmp/photos/photo-0-renamed.png");

    // Initial native listing arrives with unknown mtime and size.
    let initial = image_entry(old_path, MetadataValue::Unknown, MetadataValue::Unknown);
    let from = ids.of(&initial.location);
    let mut column = Column::new(root, false, None);
    column.apply(Edit::Replace(vec![Row::of(&initial, &ids)]));
    let old_key = column.entry(0).thumbnail.expect("image thumbnail key");

    // Load and complete the thumbnail in the window's store.
    let mut store = ThumbnailStore::default();
    let viewer = store.viewer();
    let step = Step::new(128);
    let _ = store.sync(
        viewer,
        &column,
        Viewport::new(0..1, step).listing(column.thumb_version()),
    );
    let request = store.pop(0.0).expect("initial thumbnail request");
    assert_eq!(request.key, old_key);
    let _ = store.complete(
        request.ticket,
        Outcome::Ready {
            width: 4,
            height: 4,
            rgba: vec![255; 4 * 4 * 4],
        },
        0.1,
    );
    assert!(matches!(store.state(old_key), State::Ready { .. }));

    // Metadata fill learns mtime and size while keeping `old_key` on the row.
    let filled = image_entry(
        old_path,
        MetadataValue::Known(100),
        MetadataValue::Known(512),
    );
    column.apply(Edit::Refill(vec![(0, Row::of(&filled, &ids))]));
    assert_eq!(column.entry(0).thumbnail, Some(old_key));

    // Rename arrives: `Change::Moved` before `Edit::Splice` carrying known mtime and size.
    let renamed = image_entry(
        new_path,
        MetadataValue::Known(100),
        MetadataValue::Known(512),
    );
    let to = ids.of(&renamed.location);
    let mut moving = Moving::default();
    moving.moved_in(from, to, std::slice::from_ref(&column));
    column.apply(Edit::Splice(vec![(0, 1, vec![Row::of(&renamed, &ids)])]));
    let new_key = column.entry(0).thumbnail.expect("renamed thumbnail key");
    assert_ne!(old_key, new_key);

    assert_eq!(moving.arriving(&column), vec![(from, to)]);
    let relocated = moving.take_thumbnails();
    assert_eq!(relocated, vec![(old_key, new_key)]);
    for (src, dst) in relocated {
        assert!(store.relocate(src, dst));
    }

    let _ = store.resync(viewer, &column, column.thumb_version());
    assert!(
        store.pop(0.2).is_none(),
        "renamed image must not request a new thumbnail render"
    );
    assert!(
        matches!(store.state(new_key), State::Ready { .. }),
        "renamed image retains its loaded thumbnail"
    );
}

#[test]
fn moving_skips_thumbnail_reuse_when_extension_or_facts_change() {
    let cases = [
        (
            "extension changed",
            "/tmp/photos/photo.png",
            MetadataValue::Known(100),
            MetadataValue::Known(512),
            "/tmp/photos/photo.jpg",
            MetadataValue::Known(100),
            MetadataValue::Known(512),
        ),
        (
            "mtime changed",
            "/tmp/photos/photo.png",
            MetadataValue::Known(100),
            MetadataValue::Known(512),
            "/tmp/photos/photo-2.png",
            MetadataValue::Known(200),
            MetadataValue::Known(512),
        ),
        (
            "size changed",
            "/tmp/photos/photo.png",
            MetadataValue::Known(100),
            MetadataValue::Known(512),
            "/tmp/photos/photo-2.png",
            MetadataValue::Known(100),
            MetadataValue::Known(1024),
        ),
        (
            "non-thumbnail file",
            "/tmp/photos/note.txt",
            MetadataValue::Known(100),
            MetadataValue::Known(12),
            "/tmp/photos/note-2.txt",
            MetadataValue::Known(100),
            MetadataValue::Known(12),
        ),
    ];

    for (case, old_path, old_mtime, old_size, new_path, new_mtime, new_size) in cases {
        let ids = Ids::default();
        let old_entry = image_entry(Path::new(old_path), old_mtime, old_size);
        let new_entry = image_entry(Path::new(new_path), new_mtime, new_size);
        let from = ids.of(&old_entry.location);
        let to = ids.of(&new_entry.location);

        let mut column = Column::new(Location::local(Path::new("/tmp/photos")), false, None);
        column.apply(Edit::Replace(vec![Row::of(&old_entry, &ids)]));

        let mut moving = Moving::default();
        moving.moved_in(from, to, std::slice::from_ref(&column));
        column.apply(Edit::Splice(vec![(0, 1, vec![Row::of(&new_entry, &ids)])]));
        assert_eq!(moving.arriving(&column), vec![(from, to)], "{case}");
        assert!(
            moving.take_thumbnails().is_empty(),
            "{case}: must not reuse thumbnail"
        );
    }
}
