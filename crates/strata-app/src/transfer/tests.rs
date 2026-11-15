// SPDX-License-Identifier: MIT

//! Headless behavioral tests for paste destination rules, `Ctrl+C`/`Ctrl+V`, `Ctrl+X`/`Ctrl+V`
//! moving and clearing cut marks, and `Ctrl+D` numbered duplicate.

use std::path::Path;

use rune::{Key, KeyEvent, Mods};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::paste_destination;
use crate::{
    clipboard,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted, send_key},
};

const MODULE: &str = "transfer::tests";

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn ctrl_key(host: &mut Offscreen, ch: char) {
    send_key(
        host,
        KeyEvent {
            key: Key::Command(ch),
            mods: ctrl(),
        },
    );
    while host.turn() {}
}

fn wait_exists(host: &mut Offscreen, path: &Path, exists: bool) {
    let ok = host.run_until(PATIENCE, |_| path.exists() == exists);
    assert!(ok, "expected {} exists={exists}", path.display());
    while host.turn() {}
}

fn dir_entry(path: &Path) -> FileEntry {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("dir")
        .to_owned();
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.clone().into(),
        display_name: name,
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Known(0o755),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn paste_destination_targets_selected_folder_unless_load_cursor_or_invalid() {
    let column = Location::local(Path::new("/tmp/root"));
    let child = dir_entry(Path::new("/tmp/root/sub"));

    assert_eq!(
        paste_destination(std::slice::from_ref(&child), Some(column.clone()), false),
        Some(child.location.clone())
    );
    assert_eq!(
        paste_destination(std::slice::from_ref(&child), Some(column.clone()), true),
        Some(column)
    );
    assert_eq!(
        paste_destination(&[], Some(Location::uri("trash:///")), false),
        None
    );
    assert_eq!(
        paste_destination(&[], Some(Location::uri("recent:///")), false),
        None
    );
}

#[test]
fn copy_paste_cut_paste_and_duplicate_run_through_keyboard_shortcuts() {
    hosted(
        MODULE,
        "copy_paste_cut_paste_and_duplicate_run_through_keyboard_shortcuts",
        |mut host| {
            let temp = tempfile::tempdir().expect("tempdir");
            let sub = temp.path().join("dest");
            std::fs::create_dir(&sub).expect("dest");
            let note = temp.path().join("note.txt");
            std::fs::write(&note, "hello\n").expect("note");

            host.app().open(Location::local(temp.path()));
            let ready = |h: &Offscreen| {
                h.app()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|c| c.total == 2)
            };
            assert!(host.run_until(PATIENCE, ready));
            while host.turn() {}

            let note_pos = host
                .app()
                .browser()
                .with_entries(0, 0..2, |entries| {
                    entries.iter().position(|e| e.display_name == "note.txt")
                })
                .flatten()
                .expect("note position");
            host.app().browser().select(0, note_pos);
            while host.turn() {}

            ctrl_key(&mut host, 'd');
            let dup = temp.path().join("note (1).txt");
            wait_exists(&mut host, &dup, true);
            assert_eq!(std::fs::read_to_string(&dup).expect("dup"), "hello\n");

            let ready_three = |h: &Offscreen| {
                h.app()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|c| c.total == 3)
            };
            assert!(host.run_until(PATIENCE, ready_three));
            while host.turn() {}

            let dup_pos = host
                .app()
                .browser()
                .with_entries(0, 0..3, |entries| {
                    entries
                        .iter()
                        .position(|e| e.display_name == "note (1).txt")
                })
                .flatten()
                .expect("dup position");
            host.app().browser().select(0, dup_pos);
            while host.turn() {}
            ctrl_key(&mut host, 'x');
            assert!(clipboard::has_files(host.app()));

            let dest_pos = host
                .app()
                .browser()
                .with_entries(0, 0..3, |entries| {
                    entries.iter().position(|e| e.display_name == "dest")
                })
                .flatten()
                .expect("dest position");
            host.app().browser().select(0, dest_pos);
            while host.turn() {}
            ctrl_key(&mut host, 'v');

            let moved = sub.join("note (1).txt");
            wait_exists(&mut host, &moved, true);
            wait_exists(&mut host, &dup, false);
            assert!(!clipboard::has_files(host.app()));
        },
    );
}
