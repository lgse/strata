// SPDX-License-Identifier: MIT

//! The names' find and filter highlights, read from the marks each view draws its rows with.

use std::path::Path;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use super::match_ranges;
use crate::{
    App,
    host::offscreen::Offscreen,
    icons::Icons,
    list::List,
    shell,
    tenxer::tests::{char_key, tap},
    test_support::{PATIENCE, hosted_after},
};

const MODULE: &str = "tenxer::highlight::tests";

const NAMES: [&str; 4] = ["alpha.txt", "Alphabet.md", "beta.txt", "canal.txt"];

fn in_mode(mode: &'static str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(
        MODULE,
        name,
        move || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = format!("browser_mode = \"{mode}\"\ntenxer_mode = true\n");
            std::fs::write(file, saved).expect("settings");
        },
        run,
    );
}

fn on_names(host: &mut Offscreen) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("folder");
    for name in NAMES {
        std::fs::write(root.path().join(name), name).expect("file");
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

fn press(host: &mut Offscreen, key: Key) {
    tap(host, key, Mods::default());
}

fn type_text(host: &mut Offscreen, typed: &str) {
    for character in typed.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    while host.turn() {}
}

fn find(host: &mut Offscreen, query: &str) {
    press(host, char_key("/"));
    type_text(host, query);
    press(host, Key::Enter);
}

type Pair = (usize, usize);

/// The ranges the shown view highlights in the name of `root`'s entry `name`.
fn drawn(host: &Offscreen, root: &Path, name: &str) -> Vec<Pair> {
    let app = host.app();
    let id = shell::id_of(app, &Location::local(root.join(name)));
    let results = app.filter.pane.shows();
    let marks = match shell::preferences(app).browser_mode.as_str() {
        "list" if results => crate::single::result_marks(app, id),
        "list" => crate::single::listing_marks::<List>(app, id),
        "icons" if results => crate::single::result_marks(app, id),
        "icons" => crate::single::listing_marks::<Icons>(app, id),
        _ => crate::columns::row_marks(app, 0, id),
    };
    marks
        .name_matches
        .ranges()
        .map(|range| (range.start, range.end))
        .collect()
}

fn all_drawn(host: &Offscreen, root: &Path) -> Vec<Vec<Pair>> {
    NAMES.iter().map(|name| drawn(host, root, name)).collect()
}

/// A highlighted find marks its query in every listing name, case folded; Escape takes the
/// highlight, `n` brings it back, and leaving 10xer mode drops it.
fn find_marks_the_listing(mode: &'static str, name: &str) {
    in_mode(mode, name, |mut host| {
        let root = on_names(&mut host);
        let none: Vec<Vec<Pair>> = vec![Vec::new(); NAMES.len()];
        assert_eq!(all_drawn(&host, root.path()), none);

        find(&mut host, "AL");
        let marked = vec![vec![(0, 2)], vec![(0, 2)], vec![], vec![(3, 5)]];
        assert_eq!(all_drawn(&host, root.path()), marked);

        press(&mut host, Key::Escape);
        assert_eq!(
            all_drawn(&host, root.path()),
            none,
            "Escape takes the highlight"
        );
        press(&mut host, char_key("n"));
        assert_eq!(all_drawn(&host, root.path()), marked, "n highlights again");

        let toggle = Mods {
            ctrl: true,
            cmd: true,
            shift: true,
            ..Mods::default()
        };
        tap(&mut host, Key::Command('m'), toggle);
        assert_eq!(
            all_drawn(&host, root.path()),
            none,
            "outside 10xer, no highlight"
        );
    });
}

#[test]
fn columns_find_marks_the_listing() {
    find_marks_the_listing("columns", "columns_find_marks_the_listing");
}

#[test]
fn list_find_marks_the_listing() {
    find_marks_the_listing("list", "list_find_marks_the_listing");
}

#[test]
fn icons_find_marks_the_listing() {
    find_marks_the_listing("icons", "icons_find_marks_the_listing");
}

/// `f`'s results mark what their fuzzy query matched, and a highlighted find marks its own
/// query over them instead, then over the listing once Escape dismissed the filter.
fn filter_marks_its_results(mode: &'static str, name: &str) {
    in_mode(mode, name, |mut host| {
        let root = on_names(&mut host);
        press(&mut host, char_key("f"));
        type_text(&mut host, "alp");
        press(&mut host, Key::Enter);
        let shows = |host: &Offscreen| {
            let app = host.app();
            app.columns.shows_hits(0) || app.filter.pane.shows()
        };
        assert!(
            host.run_until(PATIENCE, |h| shows(h)),
            "the filter's results show"
        );
        while host.turn() {}
        assert_eq!(drawn(&host, root.path(), "alpha.txt"), [(0, 3)]);
        assert_eq!(drawn(&host, root.path(), "Alphabet.md"), [(0, 3)]);

        find(&mut host, "ph");
        assert_eq!(
            drawn(&host, root.path(), "alpha.txt"),
            [(2, 4)],
            "the find's own"
        );

        // GTK's ladder (`keyboard/escape.rs`): the filter goes before the find's highlight.
        press(&mut host, Key::Escape);
        assert!(!shows(&host), "Escape dismisses the filter first");
        assert_eq!(drawn(&host, root.path(), "alpha.txt"), [(2, 4)]);
        press(&mut host, Key::Escape);
        assert_eq!(drawn(&host, root.path(), "alpha.txt"), Vec::<Pair>::new());
    });
}

#[test]
fn columns_filter_marks_its_results() {
    filter_marks_its_results("columns", "columns_filter_marks_its_results");
}

#[test]
fn list_filter_marks_its_results() {
    filter_marks_its_results("list", "list_filter_marks_its_results");
}

/// GTK's `match_ranges`: case folded, each match after the last, and a character whose
/// lowercase is longer keeps its own bytes.
#[test]
fn match_ranges_fold_case_and_do_not_overlap() {
    let cases: [(&str, &str, &[Pair]); 6] = [
        ("Alpha", "AL", &[(0, 2)]),
        ("aaaa", "aa", &[(0, 2), (2, 4)]),
        ("aaa", "aa", &[(0, 2)]),
        ("Écran-écran", "écr", &[(0, 4), (7, 11)]),
        ("İstanbul", "i̇s", &[(0, 3)]),
        ("beta", "", &[]),
    ];
    for (name, query, expected) in cases {
        let found: Vec<Pair> = match_ranges(name, query)
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect();
        assert_eq!(found, expected, "{name} / {query}");
    }
}
