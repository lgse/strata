// SPDX-License-Identifier: MIT

//! Headless behavioral tests for paste destination rules, `Ctrl+C`/`Ctrl+V`, `Ctrl+X`/`Ctrl+V`
//! moving and clearing cut marks, and `Ctrl+D` numbered duplicate.

use std::path::Path;

use rune::{Key, KeyEvent, Mods};
use strata_components::dialog::Tone;
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::{
    adapters::TransferTargetCollision,
    services::{KeptCause, KeptItem, RenameCause, RenamedItem, ShownName, TransferReport},
};
use strata_kit::icons::IconName;

use super::{
    begin_transfer,
    collisions::{TransferCollision, TransferPlan, transfer_conflict_text},
    dock, paste_destination,
};
use crate::{
    clipboard,
    commands::{On, Outcome, run},
    conflict_dialog::{self, ConflictFocus},
    error_dialog,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, hosted, hosted_after, send_key},
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

#[test]
fn duplicate_stops_on_filter_results_in_default_map_and_runs_in_tenxer_direct_stage() {
    hosted_after(
        MODULE,
        "duplicate_stops_on_filter_results_in_default_map_and_runs_in_tenxer_direct_stage",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("folder")).expect("settings folder");
            let saved = "tenxer_mode = false\n[keymap.tenxer]\nduplicate = [\"<Control>d\"]\n";
            std::fs::write(file, saved).expect("settings");
        },
        |mut host| {
            let temp = tempfile::tempdir().expect("tempdir");
            let beta = temp.path().join("beta.txt");
            std::fs::write(temp.path().join("alpha.txt"), "a\n").expect("alpha");
            std::fs::write(&beta, "b\n").expect("beta");
            host.app().open(Location::local(temp.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|c| c.total == 2)
            }));
            host.app().browser().select(0, 0);
            while host.turn() {}
            let beta_id = shell::id_of(host.app(), &Location::local(&beta));

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("f".to_owned()),
                    mods: ctrl(),
                },
            );
            for ch in "beta".chars() {
                send_key(
                    &mut host,
                    KeyEvent {
                        key: Key::Text(ch.to_string()),
                        mods: Mods::default(),
                    },
                );
            }
            assert!(host.run_until(PATIENCE, |h| {
                crate::filter::targets::at(h.app(), 0, beta_id).is_some()
            }));
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Down,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}

            ctrl_key(&mut host, 'd');
            assert!(host.app().transfer.preparing.is_empty());
            assert!(host.app().browser().last_started_operation().is_none());
            assert!(!host.app().browser().has_foreground_operation());
            assert!(!temp.path().join("alpha (1).txt").exists());
            assert!(!temp.path().join("beta (1).txt").exists());

            host.runtime()
                .update(|app, _| shell::saved(app).change(|s| s.tenxer_mode = true));
            while host.turn() {}
            ctrl_key(&mut host, 'd');
            wait_exists(&mut host, &temp.path().join("beta (1).txt"), true);
            assert!(!temp.path().join("alpha (1).txt").exists());
        },
    );
}

#[test]
fn mixed_collision_paste_dismissal_preserves_both_and_skip_moves_remaining() {
    hosted(
        MODULE,
        "mixed_collision_paste_dismissal_preserves_both_and_skip_moves_remaining",
        |mut host| {
            let temp = tempfile::tempdir().expect("tempdir");
            let src = temp.path().join("src");
            let dest = temp.path().join("dest");
            std::fs::create_dir(&src).expect("src");
            std::fs::create_dir(&dest).expect("dest");
            let alpha = src.join("alpha.txt");
            let beta = src.join("beta.txt");
            std::fs::write(&alpha, "source-a\n").expect("alpha");
            std::fs::write(&beta, "source-b\n").expect("beta");
            std::fs::write(dest.join("alpha.txt"), "existing-a\n").expect("existing");

            host.app().open(Location::local(&src));
            assert!(host.run_until(PATIENCE, |h| {
                h.app()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|c| c.total == 2)
            }));
            while host.turn() {}

            ctrl_key(&mut host, 'a');
            ctrl_key(&mut host, 'x');
            let dest_loc = Location::local(&dest);
            let dest_first = dest_loc.clone();
            host.runtime().update(move |app, tx| {
                assert_eq!(run(app, "paste", On::Place(dest_first), tx), Outcome::Ran);
            });
            assert!(host.run_until(PATIENCE, |h| conflict_dialog::is_open(h.app())));
            while host.turn() {}

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(!conflict_dialog::is_open(host.app()));
            assert!(!dest.join("beta.txt").exists(), "beta must not move");
            assert_eq!(
                std::fs::read_to_string(dest.join("alpha.txt")).expect("dest alpha"),
                "existing-a\n"
            );
            assert!(alpha.exists() && beta.exists());
            let alpha_id = shell::id_of(host.app(), &Location::local(&alpha));
            let beta_id = shell::id_of(host.app(), &Location::local(&beta));
            assert!(clipboard::marks(host.app(), alpha_id).cut);
            assert!(clipboard::marks(host.app(), beta_id).cut);

            // Paste again and choose Skip via Shift+Tab x2 (Replace -> Keep Both -> Skip).
            host.runtime().update(move |app, tx| {
                assert_eq!(run(app, "paste", On::Place(dest_loc), tx), Outcome::Ran);
            });
            assert!(host.run_until(PATIENCE, |h| conflict_dialog::is_open(h.app())));
            while host.turn() {}
            for _ in 0..2 {
                send_key(
                    &mut host,
                    KeyEvent {
                        key: Key::Tab,
                        mods: Mods {
                            shift: true,
                            ..Mods::default()
                        },
                    },
                );
            }
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Enter,
                    mods: Mods::default(),
                },
            );
            wait_exists(&mut host, &dest.join("beta.txt"), true);
            wait_exists(&mut host, &beta, false);
            assert!(alpha.exists(), "skipped source stays");
            assert!(clipboard::marks(host.app(), alpha_id).cut);
        },
    );
}

#[test]
fn conflict_explanations_cover_merge_unreplaceable_renamed_and_no_trash_warnings() {
    let local_dest = Location::local(Path::new("/tmp/dest"));
    let admin_dest = Location::uri("admin:///etc");

    let merge_move = transfer_conflict_text(
        &TransferCollision {
            source: Location::local(Path::new("/src/docs")),
            target_name: None,
            kinds: TransferTargetCollision::between(true, true),
            fixes_inside: true,
        },
        &TransferPlan {
            destination: local_dest.clone(),
            move_sources: true,
            reveal: true,
            focus: ConflictFocus::Replace,
        },
        true,
    );
    assert!(
        merge_move.contains("Merging combines the contents"),
        "{merge_move}"
    );
    assert!(merge_move.contains("names change to suit"), "{merge_move}");
    assert!(
        merge_move.contains("removes the source folder"),
        "{merge_move}"
    );

    let unreplaceable = transfer_conflict_text(
        &TransferCollision {
            source: Location::local(Path::new("/src/col:name")),
            target_name: Some(ShownName {
                name: "col_name".into(),
                cause: Some(RenameCause::Invalid),
            }),
            kinds: TransferTargetCollision::between(false, true),
            fixes_inside: false,
        },
        &TransferPlan {
            destination: local_dest,
            move_sources: false,
            reveal: true,
            focus: ConflictFocus::Replace,
        },
        false,
    );
    assert!(
        unreplaceable.contains("“col:name” will be saved as “col_name”"),
        "{unreplaceable}"
    );
    assert!(
        unreplaceable.contains("A file and a folder can’t replace one another."),
        "{unreplaceable}"
    );

    let no_trash = transfer_conflict_text(
        &TransferCollision {
            source: Location::local(Path::new("/src/hosts")),
            target_name: None,
            kinds: TransferTargetCollision::between(false, false),
            fixes_inside: false,
        },
        &TransferPlan {
            destination: admin_dest,
            move_sources: false,
            reveal: true,
            focus: ConflictFocus::Replace,
        },
        false,
    );
    assert!(
        no_trash.contains("\n\nReplaced items can't be moved to Trash here and will be deleted."),
        "{no_trash}"
    );
}

#[test]
fn async_paste_callback_is_scoped_to_originating_tab() {
    hosted(
        MODULE,
        "async_paste_callback_is_scoped_to_originating_tab",
        |mut host| {
            let dir1 = tempfile::tempdir().expect("dir1");
            let dir2 = tempfile::tempdir().expect("dir2");
            let note = dir1.path().join("note.txt");
            let dest1 = dir1.path().join("dest1");
            let dest2 = dir2.path().join("dest2");
            std::fs::write(&note, "tab1\n").expect("note");
            std::fs::create_dir(&dest1).expect("dest1");
            std::fs::create_dir(&dest2).expect("dest2");

            host.app().open(Location::local(dir1.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().column_entry_counts(0).is_some()
            }));
            while host.turn() {}
            let tab1_later = host.app().transfer.later.clone();

            host.runtime().update(|app, tx| {
                assert_eq!(run(app, "tab-new", On::Focus, tx), Outcome::Ran);
            });
            host.app().open(Location::local(dir2.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().active_location() == Some(Location::local(dir2.path()))
            }));
            while host.turn() {}

            let note_loc = Location::local(&note);
            let dest1_loc = Location::local(&dest1);
            host.runtime().update(move |app, tx| {
                tab1_later.then(move |app, _| {
                    begin_transfer(app, dest1_loc, vec![note_loc], false);
                    false
                });
                shell::settle(app, tx);
            });
            while host.turn() {}

            assert!(host.app().transfer.preparing.is_empty());
            assert!(!dest2.join("note.txt").exists());

            host.runtime().update(|app, tx| {
                assert_eq!(run(app, "tab-previous", On::Focus, tx), Outcome::Ran);
            });
            wait_exists(&mut host, &dest1.join("note.txt"), true);
            assert!(!dest2.join("note.txt").exists());
        },
    );
}

fn renamed_item(from: &str, to: &str, cause: RenameCause) -> RenamedItem {
    RenamedItem {
        requested: Location::local(Path::new(from)),
        saved_as: Location::local(Path::new(to)),
        cause,
    }
}

#[test]
fn present_report_distinguishes_info_renames_partial_failures_and_more_items() {
    hosted(
        MODULE,
        "present_report_distinguishes_info_renames_partial_failures_and_more_items",
        |mut host| {
            let ten_taken: Vec<RenamedItem> = (1..=10)
                .map(|i| {
                    renamed_item(
                        &format!("/d/f{i}.txt"),
                        &format!("/d/f{i} (1).txt"),
                        RenameCause::Taken,
                    )
                })
                .collect();
            let single_cause = TransferReport {
                renamed: ten_taken,
                ..TransferReport::default()
            };
            host.runtime().update(move |app, tx| {
                assert!(dock::present_report(app, &single_cause, tx));
            });
            let (icon, tone, title, summary, body) =
                error_dialog::shown_details(host.app()).expect("single-cause dialog");
            assert_eq!((icon, tone), (IconName::Info, Tone::Accent));
            assert_eq!(title, "10 items were saved under new names");
            assert_eq!(summary, Some("These names were already taken"));
            assert!(body.starts_with("• "), "{body}");
            assert!(body.contains("\n\n• "), "{body}");
            assert!(body.ends_with("\n\n… and 2 more items"), "{body}");

            let multi_cause = TransferReport {
                renamed: vec![
                    renamed_item("/d/long.txt", "/d/l.txt", RenameCause::TooLong),
                    renamed_item("/d/a.txt", "/d/a (1).txt", RenameCause::Alias),
                ],
                ..TransferReport::default()
            };
            host.runtime().update(move |app, tx| {
                assert!(dock::present_report(app, &multi_cause, tx));
            });
            let (icon, tone, _, summary, body) =
                error_dialog::shown_details(host.app()).expect("multi-cause dialog");
            assert_eq!((icon, tone), (IconName::Info, Tone::Accent));
            assert_eq!(summary, Some("Names changed for different reasons"));
            assert!(
                body.find("too long") < body.find("apart"),
                "first-appearance order: {body}"
            );

            let partial = TransferReport {
                kept: vec![KeptItem {
                    location: Location::local(Path::new("/src/locked.txt")),
                    cause: KeptCause::StillOpen(None),
                }],
                kept_out_of_undo: vec![Location::local(Path::new("/src/dir"))],
                ..TransferReport::default()
            };
            host.runtime().update(move |app, tx| {
                assert!(dock::present_report(app, &partial, tx));
            });
            let (icon, tone, title, summary, body) =
                error_dialog::shown_details(host.app()).expect("partial-failure dialog");
            assert_eq!((icon, tone), (IconName::Close, Tone::Danger));
            assert_eq!(title, "Completed with errors");
            assert_eq!(summary, Some("Some items could not be processed"));
            assert!(body.contains("Undo can’t move this item back"), "{body}");
            assert!(body.contains("• “dir”"), "{body}");
        },
    );
}
