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

    // Same-length `Edit::Replace` after `select_all`: `OverlayIds::Live` must relocate `old` to
    // `new` rather than false-positive on `new`'s post-rename index.
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

    // Length-changing `Edit::Replace` on a live range (`extend_to`): `arrived` must not pass the
    // unsynced `column` to `Selection::contains` (which would fail `source_len`'s assert).
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

/// A view in a saved mode, and the arrow that steps back from the last entry in it.
struct Moved {
    mode: &'static str,
    shape: Shape,
    back: Key,
    name: &'static str,
}

/// After the entry with the keyboard moves out of the folder, a selectionless removal, the
/// keyboard is on the row beside it with nothing selected, and the browser holds no cursor,
/// as GTK's list views leave it (finding lgse/strata#2546). The next arrow goes on from that
/// row, and the browser follows it.
fn a_move_out_leaves_the_keyboard_on_the_neighbour(case: &Moved) {
    let (shape, back) = (case.shape, case.back.clone());
    hosted_in(MODULE, case.mode, case.name, move |mut host| {
        let folder = fixture();
        let root = folder.path().to_owned();
        open(&mut host, shape, &root);
        press(&mut host, Key::End, Mods::default());
        wait(&mut host, shape, "todo.txt under the keyboard", |seen| {
            seen.focused.as_deref() == Some("todo.txt")
        });
        let (from, into) = (root.join("todo.txt"), root.join("archive"));
        host.runtime().update(move |app: &mut App, _| {
            let sources = vec![Location::local(&from)];
            crate::transfer::start_drop_transfer(app, Location::local(&into), sources, true);
        });
        let gone = host.run_until(PATIENCE, |_| root.join("archive/todo.txt").exists());
        assert!(gone, "todo.txt moved into archive");
        let seen = wait(&mut host, shape, "the keyboard on readme.md", |seen| {
            seen.focused.as_deref() == Some("readme.md")
        });
        assert!(seen.selected.is_empty(), "nothing selected: {seen:#?}");
        idle(&mut host);
        assert_eq!(
            browser_holds(&host),
            (Vec::new(), None),
            "the browser holds no cursor"
        );

        press(&mut host, back, Mods::default());
        let seen = wait(
            &mut host,
            shape,
            "the arrow to go on from readme.md",
            |seen| seen.focused.as_deref() == Some("pictures"),
        );
        assert_eq!(seen.selected, ["pictures"]);
        let holds = (vec!["pictures".to_owned()], Some("pictures".to_owned()));
        assert_eq!(browser_holds(&host), holds, "the browser follows the arrow");
    });
}

#[test]
fn a_move_out_leaves_the_keyboard_on_the_neighbour_in_icons() {
    a_move_out_leaves_the_keyboard_on_the_neighbour(&Moved {
        mode: "icons",
        shape: ICONS,
        back: Key::Left,
        name: "a_move_out_leaves_the_keyboard_on_the_neighbour_in_icons",
    });
}

#[test]
fn a_move_out_leaves_the_keyboard_on_the_neighbour_in_list() {
    a_move_out_leaves_the_keyboard_on_the_neighbour(&Moved {
        mode: "list",
        shape: LIST,
        back: Key::Up,
        name: "a_move_out_leaves_the_keyboard_on_the_neighbour_in_list",
    });
}
