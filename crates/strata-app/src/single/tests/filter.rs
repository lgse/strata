// SPDX-License-Identifier: MIT

use rune::{Key, Mods};

use super::{Case, FIXTURE, ICONS, LIST, MODULE};
use crate::test_support::{
    fixture,
    view::{assert_browser_agrees, field, hosted_in, open, press, seen, wait},
};

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

fn type_text(host: &mut crate::host::offscreen::Offscreen, typed: &str) {
    for character in typed.chars() {
        press(host, Key::Text(character.to_string()), Mods::default());
    }
}

fn results_take_the_listings_place_until_the_filter_closes(
    case: &Case,
    field_name: &str,
    name: &str,
) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::End, Mods::default());

        press(&mut host, Key::Text("f".to_owned()), ctrl());
        type_text(&mut host, "photo");
        let shown = wait(&mut host, case.shape, "the filter's one result", |seen| {
            seen.names() == ["photo.txt"]
        });
        assert_eq!(
            shown.list.as_ref().map(|[name, _]| name.as_str()),
            Some("Search results")
        );
        let tree = host.tree();
        let named = crate::test_support::view::all(&tree, &|node| {
            node["role"] == "text-input" && node["name"] == field_name
        });
        assert_eq!(named.len(), 1, "the field is named as GTK names it");
        assert_browser_agrees_with_listing(&host, "the query");

        press(&mut host, Key::Escape, Mods::default());
        let back = wait(&mut host, case.shape, "the listing back", |seen| {
            seen.names() == FIXTURE
        });
        assert_eq!(back.selected, ["todo.txt"]);
        assert_eq!(field(&host), None);
        assert_browser_agrees(&host, case.shape, "Escape");
    });
}

/// While results show, the browser keeps the listing's selection from before the filter.
fn assert_browser_agrees_with_listing(host: &crate::host::offscreen::Offscreen, after: &str) {
    let (selected, _) = crate::test_support::view::browser_holds(host);
    assert_eq!(selected, ["todo.txt"], "after {after}");
}

#[test]
fn results_take_the_grids_place_until_the_filter_closes() {
    results_take_the_listings_place_until_the_filter_closes(
        &ICONS,
        "Filter icons (Ctrl+F)",
        "filter::results_take_the_grids_place_until_the_filter_closes",
    );
}

#[test]
fn results_take_the_tables_place_until_the_filter_closes() {
    results_take_the_listings_place_until_the_filter_closes(
        &LIST,
        "Filter list (Ctrl+F)",
        "filter::results_take_the_tables_place_until_the_filter_closes",
    );
}

fn down_and_up_move_between_the_field_and_the_results(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::Text("f".to_owned()), ctrl());
        type_text(&mut host, "photo");
        wait(&mut host, case.shape, "the filter's one result", |seen| {
            seen.names() == ["photo.txt"]
        });

        press(&mut host, Key::Down, Mods::default());
        let landed = wait(
            &mut host,
            case.shape,
            "the result with the keyboard",
            |seen| seen.keyboard && seen.focused.as_deref() == Some("photo.txt"),
        );
        assert_eq!(landed.selected, ["photo.txt"]);
        assert_eq!(field(&host), Some(("photo".to_owned(), false)));

        press(&mut host, Key::Up, Mods::default());
        let reached = host.run_until(crate::test_support::PATIENCE, |host| {
            field(host) == Some(("photo".to_owned(), true))
        });
        assert!(
            reached,
            "the field did not take the keyboard: {:?}",
            field(&host)
        );
        assert_eq!(seen(&host, case.shape).names(), ["photo.txt"]);
    });
}

#[test]
fn down_and_up_move_between_the_field_and_the_tiles() {
    down_and_up_move_between_the_field_and_the_results(
        &ICONS,
        "filter::down_and_up_move_between_the_field_and_the_tiles",
    );
}

#[test]
fn down_and_up_move_between_the_field_and_the_rows() {
    down_and_up_move_between_the_field_and_the_results(
        &LIST,
        "filter::down_and_up_move_between_the_field_and_the_rows",
    );
}

fn f2_renames_the_result_under_the_cursor(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::End, Mods::default());
        press(&mut host, Key::Text("f".to_owned()), ctrl());
        type_text(&mut host, "photo");
        wait(&mut host, case.shape, "the filter's one result", |seen| {
            seen.names() == ["photo.txt"]
        });
        press(&mut host, Key::Down, Mods::default());

        press(&mut host, Key::Function(2), Mods::default());
        let editing = host.run_until(crate::test_support::PATIENCE, |host| {
            rename_field(host).as_deref() == Some("photo.txt")
        });
        assert!(
            editing,
            "no editor on the result: {:?}",
            rename_field(&host)
        );
        // The editor selects the stem, which the typing replaces.
        type_text(&mut host, "snap");
        press(&mut host, Key::Enter, Mods::default());

        let renamed = folder.path().join("pictures/snap.txt");
        let committed = host.run_until(crate::test_support::PATIENCE, |_| renamed.exists());
        assert!(committed, "the result was not renamed on disk");
        assert!(
            folder.path().join("todo.txt").exists(),
            "todo.txt was renamed"
        );
    });
}

#[test]
fn f2_renames_the_tile_under_the_cursor() {
    f2_renames_the_result_under_the_cursor(&ICONS, "filter::f2_renames_the_tile_under_the_cursor");
}

#[test]
fn f2_renames_the_row_under_the_cursor() {
    f2_renames_the_result_under_the_cursor(&LIST, "filter::f2_renames_the_row_under_the_cursor");
}

/// The rename editor's text, while it shows.
fn rename_field(host: &crate::host::offscreen::Offscreen) -> Option<String> {
    let tree = host.tree();
    let fields = crate::test_support::view::all(&tree, &|node| {
        node["role"] == "text-input" && node["name"] == "Rename"
    });
    Some(crate::test_support::view::text(fields.first()?, "value"))
}

fn filtered(
    host: &mut crate::host::offscreen::Offscreen,
    case: &Case,
    folder: &std::path::Path,
    query: &str,
) -> (Vec<String>, Vec<String>) {
    open(host, case.shape, folder);
    press(host, Key::Home, Mods::default());
    let (before, _) = crate::test_support::view::browser_holds(host);
    assert_eq!(before, ["archive"]);
    press(host, Key::Text("f".to_owned()), ctrl());
    type_text(host, query);
    let shown = wait(host, case.shape, "the results", |seen| {
        seen.list
            .as_ref()
            .is_some_and(|[name, _]| name == "Search results")
            && seen.names().len() > 2
    });
    let names = shown
        .names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    (before, names)
}

fn tab_in_a_rename_on_a_result_only_commits(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        let (before, _) = filtered(&mut host, case, folder.path(), "t");
        press(&mut host, Key::Down, Mods::default());
        let first = seen(&host, case.shape)
            .focused
            .expect("the result Down lands on");
        press(&mut host, Key::Function(2), Mods::default());
        let editing = host.run_until(crate::test_support::PATIENCE, |host| {
            rename_field(host).as_deref() == Some(first.as_str())
        });
        assert!(editing, "no editor on {first}: {:?}", rename_field(&host));
        press(&mut host, Key::End, Mods::default());
        type_text(&mut host, "x");

        // Shift+Tab: the listing under the results has an entry before the top result.
        press(&mut host, Key::Tab, crate::test_support::view::shift());

        let renamed = format!("{first}x");
        let committed = host.run_until(crate::test_support::PATIENCE, |_| {
            walkdir_has(folder.path(), &renamed)
        });
        assert!(committed, "Tab did not commit the name");
        let after = wait(
            &mut host,
            case.shape,
            "the renamed result in the pane",
            |seen| seen.names().contains(&renamed.as_str()),
        );
        assert_eq!(after.focused.as_deref(), Some(renamed.as_str()));
        assert_eq!(after.selected, [renamed]);
        crate::test_support::view::idle(&mut host);
        let (held, _) = crate::test_support::view::browser_holds(&host);
        assert_eq!(held, before, "the browser's selection changed");
    });
}

/// Whether a file named `name` is in `folder` or a folder under it.
fn walkdir_has(folder: &std::path::Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        entry.file_name() == name || (path.is_dir() && walkdir_has(&path, name))
    })
}

#[test]
fn tab_in_a_rename_on_a_tile_among_the_results_only_commits() {
    tab_in_a_rename_on_a_result_only_commits(
        &ICONS,
        "filter::tab_in_a_rename_on_a_tile_among_the_results_only_commits",
    );
}

#[test]
fn tab_in_a_rename_on_a_row_among_the_results_only_commits() {
    tab_in_a_rename_on_a_result_only_commits(
        &LIST,
        "filter::tab_in_a_rename_on_a_row_among_the_results_only_commits",
    );
}

fn a_right_click_leaves_the_results_without_an_anchor(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let names = ["a1.txt", "a2.txt", "a3.txt", "a4.txt", "b.md"];
        let folder = crate::test_support::folder_of(&names);
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::Text("f".to_owned()), ctrl());
        type_text(&mut host, "a");
        wait(&mut host, case.shape, "the four results", |seen| {
            seen.names().len() == 4
        });

        let menu = crate::test_support::view::right_click(&mut host, case.shape, "a1.txt");
        assert_eq!(menu.as_deref(), Some("a1.txt"), "no menu on the result");
        press(&mut host, Key::Escape, Mods::default());
        shift_click(&mut host, case, "a3.txt");
        assert_eq!(seen(&host, case.shape).selected, ["a3.txt"]);

        shift_click(&mut host, case, "a1.txt");
        assert_eq!(seen(&host, case.shape).selected, ["a1.txt"]);
    });
}

/// A click on the result named `name` with Shift held.
fn shift_click(host: &mut crate::host::offscreen::Offscreen, case: &Case, name: &str) {
    let tree = host.tree();
    let results = crate::test_support::view::all(&tree, &|node| {
        node["role"] == case.shape.entry && node["name"] == name
    });
    let bounds = &results.first().expect("the result")["bounds"];
    let at = |index: usize| bounds[index].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let (x, y) = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    let shift = crate::test_support::view::shift();
    host.runtime()
        .physical_input(rune::PhysicalInput::Modifiers(shift));
    crate::test_support::view::idle(host);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    host.runtime()
        .physical_input(rune::PhysicalInput::Modifiers(Mods::default()));
    crate::test_support::view::idle(host);
}

#[test]
fn a_right_click_leaves_the_tiles_without_an_anchor() {
    a_right_click_leaves_the_results_without_an_anchor(
        &ICONS,
        "filter::a_right_click_leaves_the_tiles_without_an_anchor",
    );
}

#[test]
fn a_right_click_leaves_the_rows_without_an_anchor() {
    a_right_click_leaves_the_results_without_an_anchor(
        &LIST,
        "filter::a_right_click_leaves_the_rows_without_an_anchor",
    );
}

fn escape_from_the_results_gives_the_listing_the_keyboard(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::Home, Mods::default());
        press(&mut host, Key::Text("f".to_owned()), ctrl());
        type_text(&mut host, "photo");
        wait(&mut host, case.shape, "the filter's one result", |seen| {
            seen.names() == ["photo.txt"]
        });
        press(&mut host, Key::Down, Mods::default());
        wait(
            &mut host,
            case.shape,
            "the result with the keyboard",
            |seen| seen.keyboard && seen.focused.as_deref() == Some("photo.txt"),
        );

        press(&mut host, Key::Escape, Mods::default());
        let back = wait(
            &mut host,
            case.shape,
            "the listing with the keyboard",
            |seen| seen.names() == FIXTURE && seen.keyboard,
        );
        assert_eq!(back.focused.as_deref(), Some("archive"));

        press(&mut host, case.next.clone(), Mods::default());
        assert_eq!(
            seen(&host, case.shape).focused.as_deref(),
            Some("documents")
        );
    });
}

#[test]
fn escape_from_the_tiles_gives_the_grid_the_keyboard() {
    escape_from_the_results_gives_the_listing_the_keyboard(
        &ICONS,
        "filter::escape_from_the_tiles_gives_the_grid_the_keyboard",
    );
}

#[test]
fn escape_from_the_rows_gives_the_table_the_keyboard() {
    escape_from_the_results_gives_the_listing_the_keyboard(
        &LIST,
        "filter::escape_from_the_rows_gives_the_table_the_keyboard",
    );
}

#[test]
fn directory_commands_stand_back_on_results_while_trash_runs() {
    hosted_in(
        MODULE,
        LIST.mode,
        "filter::directory_commands_stand_back_on_results_while_trash_runs",
        |mut host| {
            let folder = fixture();
            let sub = folder.path().join("pictures");
            open(&mut host, LIST.shape, &sub);
            press(&mut host, Key::Text("f".to_owned()), ctrl());
            type_text(&mut host, "photo");
            wait(&mut host, LIST.shape, "photo.txt result", |seen| {
                seen.names() == ["photo.txt"]
            });
            press(&mut host, Key::Down, Mods::default());
            wait(&mut host, LIST.shape, "photo.txt focused", |seen| {
                seen.keyboard && seen.focused.as_deref() == Some("photo.txt")
            });

            let hidden_before = host.app().browser().preferences().show_hidden;
            press(&mut host, Key::Text(".".to_owned()), ctrl());
            press(&mut host, Key::Backspace, Mods::default());
            press(
                &mut host,
                Key::Up,
                Mods {
                    alt: true,
                    ..Mods::default()
                },
            );
            assert_eq!(
                host.app().browser().preferences().show_hidden,
                hidden_before
            );
            assert_eq!(
                host.app().browser().active_location(),
                Some(strata_core::model::Location::local(&sub))
            );
        },
    );
}
