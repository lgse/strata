// SPDX-License-Identifier: MIT

use std::path::Path;

use rune::{Key, KeyEvent, Mods};
use strata_components::contract::EntryId;
use strata_core::model::Location;

use super::{
    Change, Selection, changed, complete_cut_transfer, fallback_text, has_files, is_cut_match,
    marks, may_paste, offer_serial, paths, settled_since,
};
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "clipboard::tests";

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
}

fn open_fixture(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder's listing");
    while host.turn() {}
    folder
}

fn open_two_files(host: &mut Offscreen) -> (tempfile::TempDir, Location, Location) {
    let folder = tempfile::tempdir().expect("tempdir");
    let alpha = folder.path().join("alpha file.txt");
    let beta = folder.path().join("beta.txt");
    std::fs::write(&alpha, "a\n").expect("alpha");
    std::fs::write(&beta, "b\n").expect("beta");
    host.app().open(Location::local(folder.path()));
    let listed = |h: &Offscreen| {
        h.app()
            .browser()
            .column_entry_counts(0)
            .is_some_and(|c| c.total == 2)
    };
    assert!(host.run_until(PATIENCE, listed));
    while host.turn() {}
    (folder, Location::local(&alpha), Location::local(&beta))
}

fn entry_id(app: &App, name: &str) -> EntryId {
    let count = app.browser().column_snapshot(0).expect("snapshot").count;
    let loc = app
        .browser()
        .with_entries(0, 0..count, |entries| {
            entries
                .iter()
                .find(|entry| entry.display_name == name)
                .map(|entry| entry.location.clone())
        })
        .flatten()
        .expect("entry listed");
    app.clipboard.ids.of(&loc)
}

#[test]
fn copy_path_text_escapes_shell_characters_and_appends_directory_slash() {
    let cases = [
        (Path::new("/tmp/plain.txt"), false, "/tmp/plain.txt"),
        (Path::new("/tmp/my folder"), true, "/tmp/my\\ folder/"),
        (
            Path::new("/tmp/a$b'c(d).txt"),
            false,
            "/tmp/a\\$b\\'c\\(d\\).txt",
        ),
        (
            Path::new("/tmp/line\nbreak.txt"),
            false,
            "'/tmp/line\nbreak.txt'",
        ),
    ];
    for (path, is_dir, expected) in cases {
        assert_eq!(
            paths::copy_path_text(&Location::local(path), is_dir),
            expected
        );
    }
}

#[test]
fn trash_cut_and_paste_guards_match_gtk_rules() {
    let trash_root = Location::uri("trash:///");
    let trash_item = Location::uri("trash:///note.txt");
    let nested_trash = Location::uri("trash:///folder/note.txt");
    let recent = Location::uri("recent:///");

    assert!(!paths::can_remove(&trash_root));
    assert!(paths::can_remove(&trash_item));
    assert!(!paths::can_remove(&nested_trash));

    hosted(
        MODULE,
        "trash_cut_and_paste_guards_match_gtk_rules",
        |mut host| {
            let local = Location::local(Path::new("/tmp"));
            assert!(!may_paste(host.app(), &local));

            let folder = open_fixture(&mut host);
            host.app().browser().select(0, 0);
            while host.turn() {}
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Command('c'),
                    mods: ctrl(),
                },
            );
            while host.turn() {}

            assert!(has_files(host.app()));
            assert!(may_paste(host.app(), &Location::local(folder.path())));
            assert!(!may_paste(host.app(), &trash_root));
            assert!(!may_paste(host.app(), &trash_item));
            assert!(!may_paste(host.app(), &recent));
        },
    );
}

#[test]
fn copy_and_cut_marks_track_partial_and_full_transfer_consumption() {
    hosted(
        MODULE,
        "copy_and_cut_marks_track_partial_and_full_transfer_consumption",
        |mut host| {
            let (_folder, alpha_loc, beta_loc) = open_two_files(&mut host);
            let alpha_id = entry_id(host.app(), "alpha file.txt");
            let beta_id = entry_id(host.app(), "beta.txt");

            host.runtime().update(|app, tx| {
                assert_eq!(run(app, "copy", On::Row((0, alpha_id)), tx), Outcome::Ran);
            });
            assert!(marks(host.app(), alpha_id).copied);
            assert!(!marks(host.app(), alpha_id).cut);

            host.runtime().update(|app, _| {
                app.clipboard
                    .write_files(&[alpha_loc.clone(), beta_loc.clone()], true);
                app.clipboard
                    .set_marks(Vec::new(), vec![alpha_loc.clone(), beta_loc.clone()]);
            });
            assert!(marks(host.app(), alpha_id).cut);
            assert!(marks(host.app(), beta_id).cut);
            assert!(is_cut_match(
                host.app(),
                &[beta_loc.clone(), alpha_loc.clone()]
            ));

            let moved_alpha = alpha_loc.clone();
            host.runtime().update(move |app, _| {
                complete_cut_transfer(app, std::slice::from_ref(&moved_alpha))
            });
            assert!(!marks(host.app(), alpha_id).cut);
            assert!(marks(host.app(), beta_id).cut);
            assert!(has_files(host.app()));

            let moved_beta = beta_loc.clone();
            host.runtime().update(move |app, _| {
                complete_cut_transfer(app, std::slice::from_ref(&moved_beta))
            });
            assert!(!marks(host.app(), beta_id).cut);
            assert!(!has_files(host.app()));
        },
    );
}

#[test]
fn copy_path_and_copy_name_write_text_and_clear_file_marks() {
    hosted(
        MODULE,
        "copy_path_and_copy_name_write_text_and_clear_file_marks",
        |mut host| {
            let (folder, alpha_loc, _) = open_two_files(&mut host);
            let alpha_id = entry_id(host.app(), "alpha file.txt");

            host.runtime().update(|app, tx| {
                assert_eq!(run(app, "copy", On::Row((0, alpha_id)), tx), Outcome::Ran);
                assert_eq!(
                    run(app, "copy-path", On::Row((0, alpha_id)), tx),
                    Outcome::Ran
                );
            });
            let expected = paths::copy_path_text(&alpha_loc, false);
            assert_eq!(fallback_text(host.app()), Some(expected.as_str()));
            assert!(!has_files(host.app()));
            assert!(!marks(host.app(), alpha_id).copied);

            host.runtime().update(|app, tx| {
                assert_eq!(
                    run(app, "copy-name", On::Row((0, alpha_id)), tx),
                    Outcome::Ran
                );
            });
            assert_eq!(fallback_text(host.app()), Some("alpha file.txt"));

            let folder_loc = Location::local(folder.path());
            let target = folder_loc.clone();
            host.runtime().update(move |app, tx| {
                assert_eq!(run(app, "copy-path", On::Place(target), tx), Outcome::Ran);
            });
            let expected_dir = paths::copy_path_text(&folder_loc, true);
            assert_eq!(fallback_text(host.app()), Some(expected_dir.as_str()));
        },
    );
}

fn type_filter(host: &mut Offscreen, query: &str) {
    send_key(
        host,
        KeyEvent {
            key: Key::Text("f".to_owned()),
            mods: ctrl(),
        },
    );
    for ch in query.chars() {
        send_key(
            host,
            KeyEvent {
                key: Key::Text(ch.to_string()),
                mods: Mods::default(),
            },
        );
    }
}

#[test]
fn copy_path_key_stops_on_filter_results_and_empty_results_never_copy_hidden_selection() {
    hosted(
        MODULE,
        "copy_path_key_stops_on_filter_results_and_empty_results_never_copy_hidden_selection",
        |mut host| {
            let (_folder, _, _) = open_two_files(&mut host);
            host.app().browser().select(0, 0);
            while host.turn() {}
            let beta_id = entry_id(host.app(), "beta.txt");

            type_filter(&mut host, "zzz");
            let empty_results = |h: &Offscreen| {
                crate::filter::status(h.app()).as_deref() == Some("No matching files")
            };
            assert!(host.run_until(PATIENCE, empty_results));
            host.runtime().update(|_, tx| {
                tx.focus("columns-0", true);
            });
            while host.turn() {}

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("y".to_owned()),
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert_eq!(fallback_text(host.app()), None);

            host.runtime().update(|app, tx| {
                assert_eq!(run(app, "copy-path", On::Focus, tx), Outcome::Declined);
            });
            assert_eq!(fallback_text(host.app()), None);

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            type_filter(&mut host, "beta");
            let one_hit = |h: &Offscreen| crate::filter::targets::at(h.app(), 0, beta_id).is_some();
            assert!(host.run_until(PATIENCE, one_hit));
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Down,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("y".to_owned()),
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert_eq!(fallback_text(host.app()), None);
        },
    );
}

#[test]
fn offered_clipboard_reads_update_file_availability_and_ignore_stale_revisions() {
    hosted(
        MODULE,
        "offered_clipboard_reads_update_file_availability_and_ignore_stale_revisions",
        |mut host| {
            let (_folder, _, _) = open_two_files(&mut host);
            let alpha_id = entry_id(host.app(), "alpha file.txt");

            host.runtime().update(move |app, tx| {
                assert_eq!(run(app, "copy", On::Row((0, alpha_id)), tx), Outcome::Ran);
            });
            assert!(has_files(host.app()));
            assert!(marks(host.app(), alpha_id).copied);

            host.runtime().update(|app, tx| {
                changed(
                    app,
                    &Change::Offered {
                        selection: Selection::Clipboard,
                        mimes: vec!["text/uri-list".to_owned()],
                    },
                    tx,
                );
            });
            let stale_rev = host.app().clipboard.read_revision;
            assert!(!has_files(host.app()));
            assert!(!marks(host.app(), alpha_id).copied);

            host.runtime().state.clipboard.write_text("newer text");
            host.runtime()
                .state
                .clipboard
                .apply_offered_read(stale_rev, true);
            assert!(
                !has_files(host.app()),
                "stale external file read must not override a newer text clipboard"
            );

            let before = offer_serial(host.app());
            assert!(!settled_since(host.app(), before));
            host.runtime().update(|app, tx| {
                changed(
                    app,
                    &Change::Offered {
                        selection: Selection::Clipboard,
                        mimes: vec!["x-special/gnome-copied-files".to_owned()],
                    },
                    tx,
                );
            });
            let live_rev = host.app().clipboard.read_revision;
            host.runtime().state.clipboard.pending_read = Some(live_rev);
            assert!(
                !settled_since(host.app(), before),
                "an in-flight file offer read keeps settled_since false until resolved"
            );
            host.runtime()
                .state
                .clipboard
                .apply_offered_read(live_rev, true);
            assert!(settled_since(host.app(), before));
            assert!(has_files(host.app()));

            host.runtime().update(|app, tx| {
                changed(
                    app,
                    &Change::Emptied {
                        selection: Selection::Clipboard,
                    },
                    tx,
                );
            });
            assert!(!has_files(host.app()));
        },
    );
}
