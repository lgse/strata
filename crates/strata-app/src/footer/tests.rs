// SPDX-License-Identifier: MIT

//! The footer's count and clipboard badge on the window the crate registers, read as the
//! window draws them.

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, folder_of, hosted, send_key,
        view::{all, text},
    },
};

const MODULE: &str = "footer::tests";

/// Every selected file's size known: each file holds its three-byte name.
const EVERY_SIZE: &str = "64 files selected (192 B)";

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// The count the footer draws, and its accessible description.
fn drawn_with_description(host: &Offscreen) -> Option<(String, String)> {
    let tree = host.tree();
    let count = all(&tree, &|node| node["key"] == "footer-count");
    count
        .first()
        .map(|node| (text(node, "name"), text(node, "description")))
}

/// The count the footer draws.
fn drawn(host: &Offscreen) -> Option<String> {
    drawn_with_description(host).map(|(name, _)| name)
}

fn counts_every_size(host: &Offscreen) -> bool {
    drawn(host).as_deref() == Some(EVERY_SIZE)
}

/// A short window over a folder and 64 files, with the keyboard on the first file: a range
/// from it to the last row leaves rows in the middle that no view ever shows, so only the
/// count asks for their sizes.
fn short_listing(host: &mut Offscreen) -> tempfile::TempDir {
    host.runtime().set_viewport((800.0, 300.0), 1.0);
    let names: Vec<String> = (0..64).map(|n| format!("f{n:02}")).collect();
    let folder = folder_of(&names.iter().map(String::as_str).collect::<Vec<_>>());
    std::fs::create_dir(folder.path().join("a-folder")).expect("folder");
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    press(host, Key::Down, Mods::default());
    folder
}

#[test]
fn a_selection_counts_its_files_with_the_sizes_of_rows_never_shown() {
    hosted(
        MODULE,
        "a_selection_counts_its_files_with_the_sizes_of_rows_never_shown",
        |mut host| {
            let _folder = short_listing(&mut host);
            press(&mut host, Key::End, shift());
            assert!(
                host.run_until(PATIENCE, counts_every_size),
                "the count has every selected file's size: {:?}",
                drawn(&host)
            );
        },
    );
}

#[test]
fn a_reload_while_the_sizes_are_out_asks_for_them_again() {
    hosted(
        MODULE,
        "a_reload_while_the_sizes_are_out_asks_for_them_again",
        |mut host| {
            let _folder = short_listing(&mut host);
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::End,
                    mods: shift(),
                },
            );
            // Two frames and no GLib dispatch: the count has asked, and no fill has landed.
            shell::publish(&mut host.runtime().state);
            host.runtime().frame(0.0);
            host.runtime().frame(0.0);
            host.runtime()
                .update(|app: &mut App, _: &mut Tx| app.browser().reload_active());
            assert!(
                host.run_until(PATIENCE, counts_every_size),
                "the count has every selected file's size after the reload: {:?}",
                drawn(&host)
            );
        },
    );
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

fn clipboard_badge(host: &Offscreen) -> Option<(String, String)> {
    let tree = host.tree();
    let badge = all(&tree, &|node| node["key"] == "footer-clipboard");
    badge
        .first()
        .map(|node| (text(node, "name"), text(node, "description")))
}

fn footer_present(host: &Offscreen) -> bool {
    let tree = host.tree();
    !all(&tree, &|node| node["key"] == "footer").is_empty()
}

fn run_cmd(host: &mut Offscreen, id: &'static str, on: crate::commands::On) {
    host.runtime().update(move |app, tx| {
        let _ = crate::commands::run(app, id, on, tx);
    });
    while host.turn() {}
}

fn set_paste_keys(host: &mut Offscreen, keys: &[&str]) {
    let values: Vec<String> = keys.iter().map(|s| (*s).to_owned()).collect();
    host.runtime().update(move |app, _| {
        shell::saved(app).change(|s| {
            s.keymap.default.insert("paste".to_owned(), values.into());
        });
    });
    while host.turn() {}
}

#[test]
fn clipboard_badge_follows_copy_cut_paste_and_text_copy() {
    hosted(
        MODULE,
        "clipboard_badge_follows_copy_cut_paste_and_text_copy",
        |mut host| {
            let folder = folder_of(&["note.txt"]);
            let dest = folder.path().join("dest");
            std::fs::create_dir(&dest).expect("dest folder");
            host.app().open(Location::local(folder.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().column_entry_counts(0).is_some()
            }));
            while host.turn() {}

            let note_id =
                shell::id_of(host.app(), &Location::local(folder.path().join("note.txt")));
            assert_eq!(clipboard_badge(&host), None);

            run_cmd(&mut host, "copy", crate::commands::On::Row((0, note_id)));
            assert_eq!(
                clipboard_badge(&host),
                Some((
                    "Files on clipboard".to_owned(),
                    "Press Ctrl+V to paste into a supported directory.".to_owned(),
                ))
            );

            run_cmd(
                &mut host,
                "paste",
                crate::commands::On::Place(Location::local(&dest)),
            );
            assert!(host.run_until(PATIENCE, |_| dest.join("note.txt").exists()));
            while host.turn() {}
            assert!(
                clipboard_badge(&host).is_some(),
                "pasting a copy keeps the clipboard badge"
            );
            std::fs::remove_file(dest.join("note.txt")).expect("cleanup copy");

            run_cmd(
                &mut host,
                "copy-path",
                crate::commands::On::Row((0, note_id)),
            );
            assert_eq!(
                clipboard_badge(&host),
                None,
                "copying text clears the files badge"
            );

            run_cmd(&mut host, "cut", crate::commands::On::Row((0, note_id)));
            assert!(clipboard_badge(&host).is_some());

            run_cmd(
                &mut host,
                "paste",
                crate::commands::On::Place(Location::local(&dest)),
            );
            assert!(host.run_until(PATIENCE, |_| dest.join("note.txt").exists()));
            while host.turn() {}
            assert_eq!(
                clipboard_badge(&host),
                None,
                "consuming a cut withdraws the clipboard badge"
            );
        },
    );
}

#[test]
fn clipboard_badge_keeps_footer_visible_without_hints_and_follows_paste_keymap() {
    hosted(
        MODULE,
        "clipboard_badge_keeps_footer_visible_without_hints_and_follows_paste_keymap",
        |mut host| {
            let folder = folder_of(&["alpha.txt"]);
            let alpha = Location::local(folder.path().join("alpha.txt"));
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.show_keybinding_hints = false);
            });
            while host.turn() {}
            assert!(!footer_present(&host));

            let seed = alpha.clone();
            host.runtime()
                .update(move |app, _| crate::clipboard::write_test_files(app, &[seed]));
            while host.turn() {}
            assert!(
                footer_present(&host) && clipboard_badge(&host).is_some(),
                "clipboard badge alone reveals the footer when hints and count are absent"
            );

            run_cmd(&mut host, "copy-path", crate::commands::On::Place(alpha));
            assert!(
                !footer_present(&host),
                "clearing file clipboard hides the footer again"
            );

            host.app().open(Location::local(folder.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().column_entry_counts(0).is_some()
            }));
            while host.turn() {}
            host.app().browser().select(0, 0);
            while host.turn() {}
            press(&mut host, Key::Command('c'), ctrl());

            set_paste_keys(&mut host, &["<Control><Shift>v"]);
            assert_eq!(
                clipboard_badge(&host),
                Some((
                    "Files on clipboard".to_owned(),
                    "Press Ctrl+Shift+V to paste into a supported directory.".to_owned(),
                ))
            );

            set_paste_keys(&mut host, &[]);
            assert_eq!(
                clipboard_badge(&host),
                Some(("Files on clipboard".to_owned(), String::new()))
            );
        },
    );
}

fn fixture_with_folder_and_two_files(host: &mut Offscreen) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("fixture dir");
    std::fs::write(dir.path().join("one.txt"), "one").expect("one.txt");
    std::fs::write(dir.path().join("two.txt"), "two").expect("two.txt");
    std::fs::create_dir(dir.path().join("folder")).expect("folder");
    host.app().open(Location::local(dir.path()));
    assert!(
        host.run_until(PATIENCE, |h| {
            h.app().browser().column_entry_counts(0).map(|c| c.total) == Some(3)
        }),
        "fixture folder listed"
    );
    while host.turn() {}
    dir
}

fn position_of(host: &Offscreen, name: &str) -> usize {
    (0..3)
        .find(|&pos| {
            host.app()
                .browser()
                .entry_at(0, pos)
                .is_some_and(|entry| entry.display_name == name)
        })
        .expect("entry position")
}

fn wait_count(host: &mut Offscreen, expected: &str) {
    assert!(
        host.run_until(PATIENCE, |h| drawn(h).as_deref() == Some(expected)),
        "expected footer count {expected:?}, got {:?}",
        drawn(host)
    );
}

fn wait_count_with_date(host: &mut Offscreen, prefix: &str) {
    let fallback = format!("{prefix}—");
    assert!(
        host.run_until(PATIENCE, |h| {
            drawn(h)
                .as_deref()
                .is_some_and(|s| s.starts_with(prefix) && s != fallback)
        }),
        "expected footer count starting with {prefix:?} and a filled date, got {:?}",
        drawn(host)
    );
}

#[test]
fn single_selection_mode_switches_and_empty_folder_match_gtk() {
    hosted(
        MODULE,
        "single_selection_mode_switches_and_empty_folder_match_gtk",
        |mut host| {
            let dir = fixture_with_folder_and_two_files(&mut host);
            host.app().browser().set_selection(0, &[], None);
            while host.turn() {}
            assert_eq!(
                drawn_with_description(&host),
                Some(("3 items".to_owned(), "2 files, 1 folder".to_owned()))
            );

            let folder = position_of(&host, "folder");
            let one = position_of(&host, "one.txt");
            let two = position_of(&host, "two.txt");

            host.app()
                .browser()
                .set_selection(0, &[one, two], Some(two));
            wait_count(&mut host, "2 files selected (6 B)");

            host.app().browser().set_selection(0, &[0, 1, 2], Some(2));
            wait_count(&mut host, "1 folder, 2 files selected");

            for (cmd, shows_date) in [
                ("view-columns", true),
                ("view-icons", true),
                ("view-list", false),
            ] {
                run_cmd(&mut host, cmd, crate::commands::On::Focus);
                host.app()
                    .browser()
                    .set_selection(0, &[folder], Some(folder));
                if shows_date {
                    wait_count_with_date(&mut host, "folder — ");
                } else {
                    wait_count(&mut host, "folder");
                }

                host.app().browser().set_selection(0, &[one], Some(one));
                if shows_date {
                    wait_count_with_date(&mut host, "one.txt (3 B) — ");
                } else {
                    wait_count(&mut host, "one.txt (3 B)");
                }
                assert_eq!(
                    drawn_with_description(&host).map(|(_, d)| d).as_deref(),
                    Some(
                        "1 of 3 items selected. Size includes selected files only; folder contents are not counted."
                    )
                );
            }

            // Live mode switch restyles the selected entry without reselecting.
            run_cmd(&mut host, "view-icons", crate::commands::On::Focus);
            wait_count_with_date(&mut host, "one.txt (3 B) — ");

            // Live date_format preference change updates the footer date format.
            host.runtime().update(|app, tx| {
                shell::saved(app).change(|s| s.date_format = "iso".to_owned());
                shell::sync_live_preferences(app, tx);
            });
            assert!(
                host.run_until(PATIENCE, |h| {
                    drawn(h).as_deref().is_some_and(|s| {
                        s.strip_prefix("one.txt (3 B) — ")
                            .is_some_and(|d| d.len() == 16 && d.contains('-') && d.contains(':'))
                    })
                }),
                "ISO date format should apply live: {:?}",
                drawn(&host)
            );

            // Empty folder shows "0 items".
            host.app().open(Location::local(dir.path().join("folder")));
            wait_count(&mut host, "0 items");
        },
    );
}

#[test]
fn active_filter_shows_result_count_and_escape_restores_selection_summary() {
    hosted(
        MODULE,
        "active_filter_shows_result_count_and_escape_restores_selection_summary",
        |mut host| {
            let _dir = fixture_with_folder_and_two_files(&mut host);
            let one = position_of(&host, "one.txt");
            host.app().browser().set_selection(0, &[one], Some(one));
            wait_count_with_date(&mut host, "one.txt (3 B) — ");

            run_cmd(&mut host, "filter", crate::commands::On::Focus);
            for ch in "one".chars() {
                send_key(
                    &mut host,
                    KeyEvent {
                        key: Key::Text(ch.to_string()),
                        mods: Mods::default(),
                    },
                );
            }
            wait_count(&mut host, "1 item");
            assert_eq!(
                drawn_with_description(&host).map(|(_, d)| d).as_deref(),
                Some("1 file, 0 folders")
            );

            for ch in "zzz".chars() {
                send_key(
                    &mut host,
                    KeyEvent {
                        key: Key::Text(ch.to_string()),
                        mods: Mods::default(),
                    },
                );
            }
            wait_count(&mut host, "0 items");
            assert_eq!(
                drawn_with_description(&host).map(|(_, d)| d).as_deref(),
                Some("0 files, 0 folders")
            );

            press(&mut host, Key::Escape, Mods::default());
            wait_count_with_date(&mut host, "one.txt (3 B) — ");
        },
    );
}
