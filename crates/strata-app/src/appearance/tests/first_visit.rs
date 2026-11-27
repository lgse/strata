// SPDX-License-Identifier: MIT

use rune::{Key, Mods};

use super::{
    ICONS, LIST, MODULE, alt, ctrl,
    popover::{open_popover, press, return_on, wait_closed},
};
use crate::{
    host::offscreen::Offscreen,
    test_support::{
        fixture,
        view::{browser_holds, double_click, hosted_with, idle, open, seen, wait},
    },
};

fn first_visit_to_documents(host: &mut Offscreen, folder: &std::path::Path) {
    open(host, LIST, folder);
    double_click(host, LIST, "documents");
    wait(host, LIST, "documents", |seen| seen.entries.len() == 3);
    idle(host);
}

/// A row of the Appearance popover, opened from its button.
fn choose(host: &mut Offscreen, row: &'static str) {
    open_popover(host);
    return_on(host, row);
    wait_closed(host, row);
    idle(host);
}

/// The browser's cursor alone, with nothing selected, on `name`.
fn cursor_alone(host: &Offscreen, name: &str, after: &str) {
    let held = browser_holds(host);
    assert_eq!(held, (Vec::new(), Some(name.to_owned())), "{after}");
}

#[test]
fn a_regroup_moves_the_first_visit_cursor_to_the_first_row_shown() {
    regrouping(
        "a_regroup_moves_the_first_visit_cursor_to_the_first_row_shown",
        Regrouping {
            grouped: false,
            visited: "notes.txt",
            regrouped: "spreadsheet.csv",
        },
    );
}

/// And so does an ungroup, from the first of the groups to the folder's first entry.
#[test]
fn an_ungroup_moves_the_first_visit_cursor_to_the_first_row_shown() {
    regrouping(
        "an_ungroup_moves_the_first_visit_cursor_to_the_first_row_shown",
        Regrouping {
            grouped: true,
            visited: "spreadsheet.csv",
            regrouped: "notes.txt",
        },
    );
}

struct Regrouping {
    grouped: bool,
    visited: &'static str,
    regrouped: &'static str,
}

fn regrouping(name: &str, case: Regrouping) {
    let saved = format!("group_by_type = {}\n", case.grouped);
    let name = format!("first_visit::{name}");
    hosted_with(MODULE, "list", saved, &name, |mut host| {
        let folder = fixture();
        first_visit_to_documents(&mut host, folder.path());
        cursor_alone(&host, case.visited, "the first visit");

        choose(&mut host, "appearance-group");
        cursor_alone(&host, case.regrouped, "the regroup");
        choose(&mut host, "appearance-list");
        let shown = seen(&host, LIST);
        assert_eq!(shown.focused.as_deref(), Some(case.regrouped));
        assert!(shown.keyboard, "the List row gives the view the keyboard");

        press(&mut host, Key::Down, Mods::default());
        assert_eq!(seen(&host, LIST).focused.as_deref(), Some("report.md"));
        assert_eq!(browser_holds(&host).1.as_deref(), Some("report.md"));
    });
}

#[test]
fn a_view_shown_puts_the_first_visit_cursor_on_its_first_row() {
    let name = "first_visit::a_view_shown_puts_the_first_visit_cursor_on_its_first_row";
    hosted_with(MODULE, "list", String::new(), name, |mut host| {
        let folder = fixture();
        first_visit_to_documents(&mut host, folder.path());
        choose(&mut host, "appearance-group");
        cursor_alone(&host, "spreadsheet.csv", "the regroup");

        press(&mut host, Key::Command('2'), ctrl());
        wait(&mut host, ICONS, "documents in Icons", |seen| {
            seen.entries.len() == 3
        });
        idle(&mut host);
        assert_eq!(seen(&host, ICONS).focused.as_deref(), Some("notes.txt"));
        cursor_alone(&host, "notes.txt", "Icons");
        press(&mut host, Key::Command('1'), ctrl());
        cursor_alone(&host, "notes.txt", "Columns");
        press(&mut host, Key::Command('3'), ctrl());
        wait(&mut host, LIST, "documents in List", |seen| {
            seen.entries.len() == 3
        });
        idle(&mut host);
        assert_eq!(
            seen(&host, LIST).focused.as_deref(),
            Some("spreadsheet.csv")
        );
        cursor_alone(&host, "spreadsheet.csv", "List again");
    });
}

#[test]
fn a_load_cursor_stays_through_a_regroup_and_a_switch() {
    let name = "first_visit::a_load_cursor_stays_through_a_regroup_and_a_switch";
    hosted_with(MODULE, "list", String::new(), name, |mut host| {
        let folder = fixture();
        first_visit_to_documents(&mut host, folder.path());
        press(&mut host, Key::Up, alt());
        wait(&mut host, LIST, "the root again", |seen| {
            seen.entries.len() == 5 && !seen.selected.is_empty()
        });
        idle(&mut host);
        let at_documents = (vec!["documents".to_owned()], Some("documents".to_owned()));
        assert_eq!(browser_holds(&host), at_documents, "the parent");

        choose(&mut host, "appearance-group");
        assert_eq!(browser_holds(&host), at_documents, "the regroup");
        press(&mut host, Key::Command('2'), ctrl());
        idle(&mut host);
        assert_eq!(browser_holds(&host), at_documents, "Icons");
        assert_eq!(seen(&host, ICONS).focused.as_deref(), Some("documents"));
    });
}

#[test]
fn an_entry_added_above_keeps_the_first_visit_cursor_on_its_row() {
    let name = "first_visit::an_entry_added_above_keeps_the_first_visit_cursor_on_its_row";
    hosted_with(MODULE, "list", String::new(), name, |mut host| {
        let folder = fixture();
        first_visit_to_documents(&mut host, folder.path());
        cursor_alone(&host, "notes.txt", "the first visit");

        let added = folder.path().join("documents").join("a-new.txt");
        std::fs::write(&added, "new\n").expect("a file behind the window");
        wait(&mut host, LIST, "the new file listed", |seen| {
            seen.entries.len() == 4
        });
        idle(&mut host);
        assert_eq!(seen(&host, LIST).focused.as_deref(), Some("notes.txt"));
        cursor_alone(&host, "notes.txt", "the new file");

        choose(&mut host, "appearance-group");
        cursor_alone(&host, "spreadsheet.csv", "the regroup");
    });
}
