// SPDX-License-Identifier: MIT

//! The planner, as GTK's `completion/tests.rs` tests it (ported with it), and the bar's
//! suggestions on the offscreen host: a typed path lists its folders, and the window losing
//! its activation closes them and keeps the typed text.

use std::{fs, path::Path};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;
use strata_test_support::PrivateContext;
use tempfile::tempdir;

use super::{Plan, Suggestion, matched_bytes, plan_completions};
use crate::{
    App,
    commands::{self, On},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, folder_of, hosted, send_key},
};

const MODULE: &str = "location::completion::tests";

fn suggest(input: &str, current: Option<&Path>, home: &Path, show_hidden: bool) -> Vec<Suggestion> {
    match plan_completions(input, current, home, show_hidden) {
        Plan::Ready(suggestions) => suggestions,
        Plan::List(listing) => PrivateContext::new().block_on(listing.candidates()),
    }
}

fn names(suggestions: &[Suggestion]) -> Vec<&str> {
    suggestions
        .iter()
        .map(|suggestion| suggestion.display_name.as_str())
        .collect()
}

#[test]
fn suggest_completions_for_home_shorthand() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    for folder in ["Documents", "Downloads", "Desktop", ".config"] {
        fs::create_dir_all(home.join(folder)).expect("a folder");
    }
    fs::write(home.join("notes.txt"), b"hello").expect("a file");

    assert_eq!(
        suggest("~", None, home, false),
        vec![Suggestion {
            display_name: "~/".to_owned(),
            replacement: "~/".to_owned(),
            parent_hint: "home".to_owned(),
            match_len: 1,
        }]
    );
    let home_slash = suggest("~/", None, home, false);
    assert_eq!(names(&home_slash), ["Desktop/", "Documents/", "Downloads/"]);
    assert!(names(&suggest("~/", None, home, true)).contains(&".config/"));
    assert_eq!(
        suggest("~/Doc", None, home, false),
        vec![Suggestion {
            display_name: "Documents/".to_owned(),
            replacement: "~/Documents/".to_owned(),
            parent_hint: "~".to_owned(),
            match_len: 3,
        }]
    );
    assert_eq!(
        suggest("~/.co", None, home, false),
        vec![Suggestion {
            display_name: ".config/".to_owned(),
            replacement: "~/.config/".to_owned(),
            parent_hint: "~".to_owned(),
            match_len: 3,
        }]
    );
}

#[test]
fn suggest_completions_for_absolute_paths() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path();
    fs::create_dir_all(root.join("sub/alpha")).expect("alpha");
    fs::create_dir_all(root.join("sub/beta")).expect("beta");
    fs::write(root.join("sub/albatross.txt"), b"bird").expect("a file");
    let sub = root.join("sub").to_string_lossy().into_owned();
    let dummy = Path::new("/dummy");

    assert_eq!(
        names(&suggest(&format!("{sub}/"), None, dummy, false)),
        ["alpha/", "beta/"]
    );
    assert_eq!(
        suggest(&format!("{sub}/al"), None, dummy, false),
        vec![Suggestion {
            display_name: "alpha/".to_owned(),
            replacement: format!("{sub}/alpha/"),
            parent_hint: sub.clone(),
            match_len: 2,
        }]
    );
    // A folder named in full, with no slash: its children.
    assert_eq!(
        names(&suggest(&sub, None, dummy, false)),
        ["alpha/", "beta/"]
    );
}

#[test]
fn suggest_completions_for_relative_paths() {
    let temp = tempdir().expect("tempdir");
    let current = temp.path();
    fs::create_dir_all(current.join("photos/vacation")).expect("vacation");
    fs::create_dir_all(current.join("photos/work")).expect("work");
    let dummy = Path::new("/dummy");

    let slash = suggest("photos/", Some(current), dummy, false);
    assert_eq!(names(&slash), ["vacation/", "work/"]);
    assert_eq!(
        slash[0].replacement,
        format!("{}/photos/vacation/", current.to_string_lossy())
    );
    assert_eq!(
        suggest("photos/va", Some(current), dummy, false),
        vec![Suggestion {
            display_name: "vacation/".to_owned(),
            replacement: format!("{}/photos/vacation/", current.to_string_lossy()),
            parent_hint: current.join("photos").to_string_lossy().into_owned(),
            match_len: 2,
        }]
    );
}

#[test]
fn suggest_completions_handles_empty_and_nonexistent_gracefully() {
    let temp = tempdir().expect("tempdir");
    let dummy = Path::new("/dummy");
    assert!(suggest("", None, dummy, false).is_empty());
    assert!(suggest("/definitely/not/a/real/path", None, dummy, false).is_empty());
    assert!(suggest("", Some(temp.path()), dummy, false).is_empty());
}

/// The accent covers the matched characters, a multibyte one whole.
#[test]
fn the_matched_characters_are_whole_bytes() {
    assert_eq!(matched_bytes("Écoles/", 2), Some(0..3));
    assert_eq!(matched_bytes("docs/", 0), None);
}

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn press(host: &mut Offscreen, key: Key) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods::default(),
        },
    );
    idle(host);
}

/// Opens a folder with two folders in it, then the location entry, and types `text`.
fn typed(host: &mut Offscreen, text: impl Fn(&Path) -> String) -> tempfile::TempDir {
    let folder = folder_of(&["one.txt"]);
    for name in ["documents", "downloads"] {
        fs::create_dir(folder.path().join(name)).expect("a folder");
    }
    host.app().open(Location::local(folder.path()));
    let named = host.run_until(PATIENCE, |host| host.app().location.shown.is_some());
    assert!(named, "the bar named the folder");
    idle(host);
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        commands::run(app, "location", On::Focus, tx);
    });
    idle(host);
    for character in text(folder.path()).chars() {
        press(host, Key::Text(character.to_string()));
    }
    folder
}

fn suggested(host: &Offscreen) -> Vec<String> {
    let state = &host.app().location.state;
    state
        .suggestions()
        .iter()
        .map(|candidate| candidate.label.clone())
        .collect()
}

/// A typed path lists the folders that match its leaf, off Rune's thread.
#[test]
fn a_typed_path_lists_its_folders() {
    hosted(MODULE, "a_typed_path_lists_its_folders", |mut host| {
        let _folder = typed(&mut host, |folder| format!("{}/doc", folder.display()));
        let listed = host.run_until(PATIENCE, |host| suggested(host) == ["documents/"]);
        assert!(listed, "the suggestions: {:?}", suggested(&host));
        assert!(host.app().location.state.suggestions_open());
    });
}

/// The window losing its activation closes the suggestions and keeps the typed text, as GTK's
/// `set_window_presence` does.
#[test]
fn the_window_losing_activation_closes_the_suggestions() {
    let name = "the_window_losing_activation_closes_the_suggestions";
    hosted(MODULE, name, |mut host| {
        let folder = typed(&mut host, |folder| format!("{}/d", folder.display()));
        let listed = host.run_until(PATIENCE, |host| suggested(host).len() == 2);
        assert!(listed, "the suggestions: {:?}", suggested(&host));

        host.runtime().window_focus(false);
        idle(&mut host);

        let state = &host.app().location.state;
        assert!(!state.suggestions_open(), "the suggestions closed");
        assert_eq!(
            state.edit.text,
            format!("{}/d", folder.path().display()),
            "the typed text stays"
        );
    });
}
