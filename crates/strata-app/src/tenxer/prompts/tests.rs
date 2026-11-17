// SPDX-License-Identifier: MIT

//! 10xer's footer prompts and file verbs on the offscreen host: find and repeat, the filter's
//! `/` descent, create and rename by typed path, delete confirmation, and yank and paste.

use std::path::Path;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use super::{PromptKind, slash_over};
use crate::{
    App, clipboard,
    host::offscreen::Offscreen,
    shell,
    tenxer::tests::{char_key, node_name, tap},
    test_support::{
        PATIENCE, hosted, hosted_after,
        view::{all, text},
    },
};

const MODULE: &str = "tenxer::prompts::tests";

fn type_text(host: &mut Offscreen, typed: &str) {
    for character in typed.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    while host.turn() {}
}

fn press(host: &mut Offscreen, key: Key) {
    tap(host, key, Mods::default());
}

/// A capital letter, as a layout sends it with Shift held.
fn shifted(host: &mut Offscreen, capital: &str) {
    let mods = Mods {
        shift: true,
        ..Mods::default()
    };
    tap(host, char_key(capital), mods);
}

/// A 10xer window on a new folder of `files` and `folders`, the keyboard in its listing.
fn tenxer_on(host: &mut Offscreen, files: &[&str], folders: &[&str]) -> tempfile::TempDir {
    host.runtime().update(|app: &mut App, _: &mut Tx| {
        shell::saved(app).change(|settings| settings.tenxer_mode = true);
    });
    let root = tempfile::tempdir().expect("folder");
    for name in files {
        std::fs::write(root.path().join(name), name).expect("file");
    }
    for name in folders {
        std::fs::create_dir(root.path().join(name)).expect("folder");
    }
    host.app().open(Location::local(root.path()));
    assert!(host.run_until(PATIENCE, |h| {
        h.app().browser().column_entry_counts(0).is_some()
    }));
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    root
}

fn cursor(host: &Offscreen) -> Option<usize> {
    host.app().browser().focused_item().map(|(_, at, _)| at)
}

fn flash(host: &Offscreen) -> Option<String> {
    node_name(host, "footer-flash")
}

fn prompt(host: &Offscreen) -> Option<(PromptKind, String)> {
    let state = &host.app().tenxer.prompt;
    state.kind().map(|kind| (kind, state.text().to_owned()))
}

fn dialog(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    all(&tree, &|node| node["role"] == "dialog")
        .first()
        .map(|node| text(node, "name"))
}

fn shows_hits(host: &Offscreen) -> bool {
    let app = host.app();
    app.columns.shows_hits(0) || app.filter.pane.shows()
}

fn wait_on_disk(host: &mut Offscreen, path: &Path) {
    assert!(
        host.run_until(PATIENCE, |_| path.exists()),
        "{} never appeared",
        path.display()
    );
}

#[test]
fn a_typed_slash_enters_the_folder_only_when_it_is_the_one_insertion() {
    let cases = [
        ("doc", "doc/", Some("doc")),
        ("doc", "d/oc", Some("doc")),
        ("", "/", None),
        ("  ", "  /", None),
        ("doc", "doc//", None),
        ("doc", "docs", None),
        ("doc", "do", None),
    ];
    for (before, after, entered) in cases {
        assert_eq!(
            slash_over(before, after),
            entered,
            "{before:?} -> {after:?}"
        );
    }
}

#[test]
fn find_moves_only_the_cursor_and_n_repeats_both_ways() {
    hosted(
        MODULE,
        "find_moves_only_the_cursor_and_n_repeats_both_ways",
        |mut host| {
            let names = ["alpha.txt", "alphabet.md", "beta.txt", "gamma.txt"];
            let _root = tenxer_on(&mut host, &names, &[]);
            press(&mut host, char_key("n"));
            assert_eq!(flash(&host).as_deref(), Some("No previous find"));

            host.app().browser().select(0, 2);
            while host.turn() {}
            press(&mut host, char_key("/"));
            assert_eq!(prompt(&host).map(|(kind, _)| kind), Some(PromptKind::Find));
            type_text(&mut host, "ALPHA");
            press(&mut host, Key::Enter);
            assert_eq!(prompt(&host), None, "Enter closes the prompt");
            assert_eq!(cursor(&host), Some(0), "the next match wraps past the end");

            press(&mut host, char_key("n"));
            assert_eq!(cursor(&host), Some(1));
            press(&mut host, char_key("n"));
            assert_eq!(cursor(&host), Some(0), "n wraps");
            shifted(&mut host, "N");
            assert_eq!(cursor(&host), Some(1), "N searches backward");
            assert_eq!(host.app().browser().selected_positions(0), vec![2]);

            press(&mut host, char_key("/"));
            type_text(&mut host, "zz");
            press(&mut host, Key::Enter);
            assert_eq!(cursor(&host), Some(1), "a miss keeps the cursor");
            assert_eq!(
                flash(&host).as_deref(),
                Some("No matches for \u{201c}zz\u{201d}")
            );
        },
    );
}

#[test]
fn a_slash_in_the_filter_enters_the_settled_folder_and_filters_it_anew() {
    hosted(
        MODULE,
        "a_slash_in_the_filter_enters_the_settled_folder_and_filters_it_anew",
        |mut host| {
            let root = tenxer_on(&mut host, &["readme.md"], &["documents"]);
            press(&mut host, char_key("f"));
            type_text(&mut host, "documents");
            assert_eq!(dialog(&host), None, "the prompt keeps d and its verbs");
            type_text(&mut host, "/");
            let inside = Location::local(root.path().join("documents"));
            let entered = host.run_until(PATIENCE, |h| {
                h.app().browser().active_location().as_ref() == Some(&inside)
            });
            assert!(entered, "the settled folder match opens");
            while host.turn() {}
            assert_eq!(prompt(&host), Some((PromptKind::Filter, String::new())));
        },
    );
}

#[test]
fn create_and_rename_take_typed_paths_relative_to_the_listing() {
    hosted(
        MODULE,
        "create_and_rename_take_typed_paths_relative_to_the_listing",
        |mut host| {
            let root = tenxer_on(&mut host, &["alpha.txt"], &["sub"]);
            press(&mut host, char_key("a"));
            type_text(&mut host, "made/");
            press(&mut host, Key::Enter);
            wait_on_disk(&mut host, &root.path().join("made"));
            assert!(
                root.path().join("made").is_dir(),
                "a trailing / makes a folder"
            );

            press(&mut host, char_key("a"));
            type_text(&mut host, "note.txt");
            press(&mut host, Key::Enter);
            wait_on_disk(&mut host, &root.path().join("note.txt"));
            assert!(root.path().join("note.txt").is_file());

            let alpha = Location::local(root.path().join("alpha.txt"));
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let id = shell::id_of(app, &alpha);
                let view = shell::shown_view(app).expect("a view");
                (view.rows.select_only)(app, (0, id), tx);
            });
            while host.turn() {}
            press(&mut host, char_key("r"));
            let opened = prompt(&host).expect("the rename prompt");
            assert_eq!(opened, (PromptKind::Rename, "alpha.txt".to_owned()));
            type_text(&mut host, "sub/beta");
            press(&mut host, Key::Enter);
            wait_on_disk(&mut host, &root.path().join("sub/beta.txt"));
            assert!(
                !root.path().join("alpha.txt").exists(),
                "the stem was replaced"
            );
        },
    );
}

#[test]
fn d_confirms_a_trash_dialog_but_never_a_permanent_delete() {
    hosted(
        MODULE,
        "d_confirms_a_trash_dialog_but_never_a_permanent_delete",
        |mut host| {
            let root = tenxer_on(&mut host, &["alpha.txt"], &[]);
            let alpha = root.path().join("alpha.txt");
            host.app().browser().select(0, 0);
            while host.turn() {}

            shifted(&mut host, "D");
            let permanent = dialog(&host).expect("the permanent delete dialog");
            assert!(permanent.starts_with("Permanently delete"), "{permanent}");
            press(&mut host, char_key("d"));
            assert_eq!(dialog(&host), Some(permanent), "d leaves it open");
            assert!(alpha.exists());
            press(&mut host, Key::Escape);
            assert!(host.run_until(PATIENCE, |h| dialog(h).is_none()));

            press(&mut host, char_key("d"));
            let trash = dialog(&host).expect("the trash dialog");
            assert!(trash.ends_with("to Trash?"), "{trash}");
            press(&mut host, char_key("d"));
            assert!(
                host.run_until(PATIENCE, |_| !alpha.exists()),
                "the second d trashes"
            );
        },
    );
}

#[test]
fn yank_fills_the_clipboard_unyank_empties_it_and_an_empty_paste_says_so() {
    hosted(
        MODULE,
        "yank_fills_the_clipboard_unyank_empties_it_and_an_empty_paste_says_so",
        |mut host| {
            let _root = tenxer_on(&mut host, &["alpha.txt"], &[]);
            press(&mut host, char_key("p"));
            assert_eq!(flash(&host).as_deref(), Some("Nothing to paste"));

            host.app().browser().select(0, 0);
            while host.turn() {}
            press(&mut host, char_key("y"));
            assert!(
                host.run_until(PATIENCE, |h| clipboard::has_files(h.app())),
                "y copies the files"
            );
            shifted(&mut host, "Y");
            assert!(!clipboard::has_files(host.app()), "Y releases them");

            shifted(&mut host, "R");
            assert_eq!(
                flash(&host).as_deref(),
                Some("Only items in Trash can be restored")
            );
        },
    );
}

#[test]
fn leaving_tenxer_mode_closes_the_prompt_and_its_filter() {
    hosted(
        MODULE,
        "leaving_tenxer_mode_closes_the_prompt_and_its_filter",
        |mut host| {
            let _root = tenxer_on(&mut host, &["alpha.txt", "beta.txt"], &[]);
            press(&mut host, char_key("f"));
            type_text(&mut host, "al");
            press(&mut host, Key::Enter);
            assert!(shows_hits(&host), "Enter commits the filter");
            press(&mut host, char_key("f"));
            assert_eq!(prompt(&host), Some((PromptKind::Filter, "al".to_owned())));

            let toggle = Mods {
                ctrl: true,
                cmd: true,
                shift: true,
                ..Mods::default()
            };
            tap(&mut host, Key::Command('m'), toggle);
            assert!(!shell::preferences(host.app()).tenxer_mode);
            assert_eq!(prompt(&host), None);
            assert!(!shows_hits(&host), "10xer's filter goes too");
        },
    );
}

/// In List, where no preview follows the selection, so each Escape has one thing to dismiss.
#[test]
fn escape_dismisses_the_find_highlight_before_the_selection() {
    hosted_after(
        MODULE,
        "escape_dismisses_the_find_highlight_before_the_selection",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            std::fs::write(file, "browser_mode = \"list\"\n").expect("settings");
        },
        |mut host| {
            let _root = tenxer_on(&mut host, &["alpha.txt", "beta.txt", "gamma.txt"], &[]);
            let selected = |host: &Offscreen| host.app().browser().selected_positions(0);
            host.app().browser().select(0, 2);
            while host.turn() {}
            press(&mut host, char_key("/"));
            type_text(&mut host, "alpha");
            press(&mut host, Key::Enter);
            press(&mut host, Key::Escape);
            assert_eq!(
                selected(&host),
                vec![2],
                "the first Escape takes the highlight"
            );
            press(&mut host, Key::Escape);
            assert!(
                selected(&host).is_empty(),
                "the second clears the selection"
            );

            host.app().browser().select(0, 2);
            while host.turn() {}
            press(&mut host, char_key("n"));
            press(&mut host, Key::Escape);
            assert_eq!(selected(&host), vec![2], "n highlights the matches again");

            press(&mut host, char_key("n"));
            press(&mut host, char_key("/"));
            press(&mut host, Key::Escape);
            assert_eq!(prompt(&host), None, "Escape closes the find prompt");
            press(&mut host, Key::Escape);
            assert!(
                selected(&host).is_empty(),
                "the prompt's Escape took the highlight"
            );
        },
    );
}
