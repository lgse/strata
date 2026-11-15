// SPDX-License-Identifier: MIT

//! Behavioral tests for clipboard path escaping, trash cut guards, row marks, and partial vs
//! full cut consumption.

use std::path::Path;

use rune::{Key, KeyEvent, Mods};
use strata_components::contract::EntryId;
use strata_core::model::Location;

use super::{
    complete_cut_transfer, fallback_text, has_files, is_cut_match, marks, may_paste, paths,
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
