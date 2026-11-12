// SPDX-License-Identifier: MIT

//! The single-pane views on the offscreen host, over real folders, read from the
//! accessibility tree as the QA harness reads it. Each behaviour runs once for each view,
//! with what differs between them as data: a test runs in a process of its own, so each
//! view's case is a test of its own.

use rune::{Key, Mods};
use strata_core::model::Location;

use crate::test_support::{
    PATIENCE, fixture,
    view::{Shape, assert_browser_agrees, field, hosted_in, open, press, seen, shift, wait},
};

const MODULE: &str = "single::tests";
const FIXTURE: [&str; 5] = ["archive", "documents", "pictures", "readme.md", "todo.txt"];

/// What differs between the views in these tests.
struct Case {
    /// The saved `browser_mode` it opens on.
    mode: &'static str,
    shape: Shape,
    /// The key to the next entry and to the previous one.
    next: Key,
    previous: Key,
    /// A sideways key that stays on the entry, where the view has one.
    stays: Option<Key>,
}

const ICONS: Case = Case {
    mode: "icons",
    shape: Shape {
        described: "Icons view",
        list: "grid",
        entry: "grid-cell",
    },
    next: Key::Right,
    previous: Key::Left,
    stays: None,
};

/// List rows have no sideways neighbours: Right stays (`horizontal_arrow_action` in
/// `src/ui/window/key_routing.rs`).
const LIST: Case = Case {
    mode: "list",
    shape: Shape {
        described: "List view",
        list: "grid",
        entry: "row",
    },
    next: Key::Down,
    previous: Key::Up,
    stays: Some(Key::Right),
};

/// The window opens on the view the saved mode names, under the name the GTK application
/// read before the setting was renamed too, named as GTK names the pane, the list and the
/// entries (`src/ui/accessibility.rs`).
fn opens_named_as_gtk_names_it(case: &Case, mode: &'static str, name: &str) {
    hosted_in(MODULE, mode, name, |mut host| {
        let folder = fixture();
        let name = Location::local(folder.path()).display_name();

        let seen = open(&mut host, case.shape, folder.path());

        let described = case.shape.described.to_owned();
        assert_eq!(seen.pane, Some([name.clone(), described]));
        assert_eq!(seen.list, Some([name, "Files".to_owned()]));
        assert_eq!(seen.names(), FIXTURE);
        let kinds: Vec<&str> = seen.entries.iter().map(|[_, kind]| kind.as_str()).collect();
        assert_eq!(kinds, ["Folder", "Folder", "Folder", "File", "File"]);
        assert_eq!(seen.focused.as_deref(), Some("archive"));
        assert_browser_agrees(&host, case.shape, "the launch");
    });
}

#[test]
fn a_saved_grid_mode_opens_the_icons_view_named_as_gtk_names_it() {
    let name = "a_saved_grid_mode_opens_the_icons_view_named_as_gtk_names_it";
    opens_named_as_gtk_names_it(&ICONS, "grid", name);
}

#[test]
fn a_saved_explorer_mode_opens_the_list_view_named_as_gtk_names_it() {
    let name = "a_saved_explorer_mode_opens_the_list_view_named_as_gtk_names_it";
    opens_named_as_gtk_names_it(&LIST, "explorer", name);
}

/// The keys of `keyboard_navigation`'s runs (`arrow-keys-move-focus-remaining`,
/// `shift-arrow-extends-selection`): the view moves the cursor and the selection, and after
/// every key the browser holds what the view shows.
fn keys_move_and_the_browser_holds_the_result(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::End, Mods::default());
        press(&mut host, case.previous.clone(), Mods::default());

        let none = Mods::default();
        let mut steps: Vec<(Key, Mods, &str, &[&str])> = vec![
            (case.next.clone(), none, "todo.txt", &["todo.txt"]),
            (case.previous.clone(), none, "readme.md", &["readme.md"]),
        ];
        if let Some(stays) = &case.stays {
            steps.push((stays.clone(), none, "readme.md", &["readme.md"]));
        }
        steps.push((
            case.next.clone(),
            shift(),
            "todo.txt",
            &["readme.md", "todo.txt"],
        ));
        steps.push((case.previous.clone(), shift(), "readme.md", &["readme.md"]));
        for (key, mods, focused, selected) in steps {
            let after = format!("{key:?} with {mods:?}");
            press(&mut host, key, mods);
            let seen = seen(&host, case.shape);
            assert_eq!(seen.focused.as_deref(), Some(focused), "{after}: {seen:#?}");
            assert_eq!(seen.selected, selected, "{after}");
            assert_browser_agrees(&host, case.shape, &after);
        }
    });
}

#[test]
fn keys_move_in_the_grid_and_the_browser_holds_the_result() {
    let name = "keys_move_in_the_grid_and_the_browser_holds_the_result";
    keys_move_and_the_browser_holds_the_result(&ICONS, name);
}

#[test]
fn keys_move_in_the_table_and_the_browser_holds_the_result() {
    let name = "keys_move_in_the_table_and_the_browser_holds_the_result";
    keys_move_and_the_browser_holds_the_result(&LIST, name);
}

/// Return on a folder opens it in place: one pane, of the folder's entries, and the browser
/// works in it, as GTK's Icons and List views do (`activate_focused_in_place`).
fn return_opens_a_folder_in_place(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, case.next.clone(), Mods::default());

        press(&mut host, Key::Enter, Mods::default());

        let seen = wait(&mut host, case.shape, "the folder to open", |seen| {
            seen.pane
                .as_ref()
                .is_some_and(|[name, _]| name == "documents")
        });
        assert_eq!(seen.names(), ["notes.txt", "report.md", "spreadsheet.csv"]);
        let browser = host.app().browser();
        assert_eq!(browser.location_at(1), None, "a single column");
        let active = browser
            .active_location()
            .map(|location| location.display_name());
        assert_eq!(active.as_deref(), Some("documents"));
    });
}

#[test]
fn return_opens_a_folder_in_place_in_icons() {
    return_opens_a_folder_in_place(&ICONS, "return_opens_a_folder_in_place_in_icons");
}

#[test]
fn return_opens_a_folder_in_place_in_list() {
    return_opens_a_folder_in_place(&LIST, "return_opens_a_folder_in_place_in_list");
}

/// `inline_renaming`'s runs: F2 opens the editor on the cursor's entry with the keyboard in
/// it, and a window key the field passes on leaves it there.
fn f2_opens_the_editor_and_f5_leaves_it(case: &Case, name: &str) {
    hosted_in(MODULE, case.mode, name, |mut host| {
        let folder = fixture();
        open(&mut host, case.shape, folder.path());
        press(&mut host, Key::End, Mods::default());

        press(&mut host, Key::Function(2), Mods::default());
        let editing = host.run_until(PATIENCE, |host| {
            field(host).is_some_and(|(_, focused)| focused)
        });
        assert!(editing, "the field: {:?}", field(&host));
        assert_eq!(
            field(&host).map(|(text, _)| text).as_deref(),
            Some("todo.txt")
        );

        press(&mut host, Key::Function(5), Mods::default());
        host.run_until(std::time::Duration::from_millis(500), |_| false);
        assert_eq!(field(&host).map(|(_, focused)| focused), Some(true));
    });
}

#[test]
fn f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there_in_icons() {
    let name = "f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there_in_icons";
    f2_opens_the_editor_and_f5_leaves_it(&ICONS, name);
}

#[test]
fn f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there_in_list() {
    let name = "f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there_in_list";
    f2_opens_the_editor_and_f5_leaves_it(&LIST, name);
}
