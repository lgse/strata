// SPDX-License-Identifier: MIT

//! The Icons view on the offscreen host, over real folders, read from the accessibility tree
//! as the QA harness reads it.

use std::path::{Path, PathBuf};

use rune::{Key, KeyEvent, Mods, PhysicalInput};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted_after},
};

const MODULE: &str = "icons::tests";
const FIXTURE: [&str; 5] = ["archive", "documents", "pictures", "readme.md", "todo.txt"];

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

/// Runs `run` in a window whose saved `browser_mode` is `mode`.
fn hosted_in(mode: &'static str, name: &str, run: impl FnOnce(Offscreen)) {
    let prepare = move || {
        let file = settings_file();
        std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
        let saved =
            format!("mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"{mode}\"\n");
        std::fs::write(file, saved).expect("settings");
    };
    hosted_after(MODULE, name, prepare, run);
}

/// The Icons view as assistive technology hears it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    /// The group around the grid: its name and description.
    pane: Option<[String; 2]>,
    /// The grid: its name and description.
    grid: Option<[String; 2]>,
    /// Each cell's name and description.
    cells: Vec<[String; 2]>,
    selected: Vec<String>,
    /// The grid's active descendant: where the keyboard is.
    focused: Option<String>,
}

fn text(node: &Value, field: &str) -> String {
    node[field].as_str().unwrap_or_default().to_owned()
}

fn heard(node: &Value) -> [String; 2] {
    [text(node, "name"), text(node, "description")]
}

fn find<'a>(node: &'a Value, wanted: &dyn Fn(&Value) -> bool, found: &mut Vec<&'a Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if wanted(child) {
            found.push(child);
        }
        find(child, wanted, found);
    }
}

fn all<'a>(node: &'a Value, wanted: &dyn Fn(&Value) -> bool) -> Vec<&'a Value> {
    let mut found = Vec::new();
    find(node, wanted, &mut found);
    found
}

fn seen(host: &Offscreen) -> Seen {
    let tree = host.tree();
    let pane = all(&tree, &|node| node["description"] == "Icons view");
    let Some(pane) = pane.first() else {
        return Seen::default();
    };
    let grid = all(pane, &|node| node["role"] == "grid");
    let Some(grid) = grid.first() else {
        return Seen::default();
    };
    let cells = all(grid, &|node| node["role"] == "grid-cell");
    let selected = |node: &Value| {
        let states = node["states"].as_array();
        states.is_some_and(|states| states.iter().any(|state| state == "selected"))
    };
    let active = &grid["activeDescendant"];
    Seen {
        pane: Some(heard(pane)),
        grid: Some(heard(grid)),
        cells: cells.iter().map(|cell| heard(cell)).collect(),
        selected: cells
            .iter()
            .filter(|cell| selected(cell))
            .map(|cell| text(cell, "name"))
            .collect(),
        focused: cells
            .iter()
            .find(|cell| cell["key"] == *active)
            .map(|cell| text(cell, "name")),
    }
}

fn names(seen: &Seen) -> Vec<&str> {
    seen.cells.iter().map(|[name, _]| name.as_str()).collect()
}

/// Turns the host until neither GLib nor Rune has anything ready, landing each motion.
fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn wait(host: &mut Offscreen, what: &str, done: impl Fn(&Seen) -> bool) -> Seen {
    let reached = host.run_until(PATIENCE, |host| done(&seen(host)));
    let seen = seen(host);
    assert!(reached, "waited for {what}; the tree shows {seen:#?}");
    seen
}

fn open(host: &mut Offscreen, folder: &Path) -> Seen {
    host.app().open(Location::local(folder));
    let listed = wait(host, "the folder's listing", |seen| !seen.cells.is_empty());
    idle(host);
    listed
}

/// Feeds the host a key with its modifiers held, as a keyboard sends one.
fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    let runtime = host.runtime();
    runtime.physical_input(PhysicalInput::Modifiers(mods));
    runtime.key(KeyEvent { key, mods });
    runtime.physical_input(PhysicalInput::Modifiers(Mods::default()));
    idle(host);
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// What the browser holds: the names it has selected, and its focused entry's.
fn browser_holds(host: &Offscreen) -> (Vec<String>, Option<String>) {
    let browser = host.app().browser();
    let selected = browser.selected_entries();
    let selected = selected.into_iter().map(|entry| entry.display_name);
    let focused = browser.focused_entry().map(|entry| entry.display_name);
    (selected.collect(), focused)
}

fn assert_browser_agrees(host: &Offscreen, after: &str) {
    let seen = seen(host);
    assert_eq!(
        browser_holds(host),
        (seen.selected, seen.focused),
        "after {after}"
    );
}

/// `grid` is what the GTK application read as Icons before the setting was renamed, and the
/// window opens on the Icons view for it, named as GTK names the pane, the grid and the
/// cells (`src/ui/accessibility.rs`).
#[test]
fn a_saved_grid_mode_opens_the_icons_view_named_as_gtk_names_it() {
    hosted_in(
        "grid",
        "a_saved_grid_mode_opens_the_icons_view_named_as_gtk_names_it",
        |mut host| {
            let folder = fixture();
            let name = Location::local(folder.path()).display_name();

            let seen = open(&mut host, folder.path());

            assert_eq!(seen.pane, Some([name.clone(), "Icons view".to_owned()]));
            assert_eq!(seen.grid, Some([name, "Files".to_owned()]));
            assert_eq!(names(&seen), FIXTURE);
            let described: Vec<&str> = seen.cells.iter().map(|[_, kind]| kind.as_str()).collect();
            assert_eq!(described, ["Folder", "Folder", "Folder", "File", "File"]);
            assert_eq!(seen.focused.as_deref(), Some("archive"));
            assert_browser_agrees(&host, "the launch");
        },
    );
}

/// The keys of `keyboard_navigation`'s Icons runs (`arrow-keys-move-focus-remaining`,
/// `shift-arrow-extends-selection`): the grid moves the cursor and the selection, and after
/// every key the browser holds what the grid shows.
#[test]
fn keys_move_in_the_grid_and_the_browser_holds_the_result() {
    hosted_in(
        "icons",
        "keys_move_in_the_grid_and_the_browser_holds_the_result",
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            press(&mut host, Key::End, Mods::default());
            press(&mut host, Key::Left, Mods::default());

            let steps: [(Key, Mods, &str, &[&str]); 4] = [
                (Key::Right, Mods::default(), "todo.txt", &["todo.txt"]),
                (Key::Left, Mods::default(), "readme.md", &["readme.md"]),
                (Key::Right, shift(), "todo.txt", &["readme.md", "todo.txt"]),
                (Key::Left, shift(), "readme.md", &["readme.md"]),
            ];
            for (key, mods, focused, selected) in steps {
                let after = format!("{key:?} with {mods:?}");
                press(&mut host, key, mods);
                let seen = seen(&host);
                assert_eq!(seen.focused.as_deref(), Some(focused), "{after}: {seen:#?}");
                assert_eq!(seen.selected, selected, "{after}");
                assert_browser_agrees(&host, &after);
            }
        },
    );
}

/// Return on a folder opens it in place: one grid, of the folder's entries, and the browser
/// works in it, as GTK's Icons view does (`activate_focused_in_place`).
#[test]
fn return_opens_a_folder_in_place() {
    hosted_in("icons", "return_opens_a_folder_in_place", |mut host| {
        let folder = fixture();
        open(&mut host, folder.path());
        press(&mut host, Key::Right, Mods::default());

        press(&mut host, Key::Enter, Mods::default());

        let seen = wait(&mut host, "the folder to open", |seen| {
            seen.pane
                .as_ref()
                .is_some_and(|[name, _]| name == "documents")
        });
        assert_eq!(names(&seen), ["notes.txt", "report.md", "spreadsheet.csv"]);
        let browser = host.app().browser();
        assert_eq!(browser.location_at(1), None, "a single column");
        let active = browser
            .active_location()
            .map(|location| location.display_name());
        assert_eq!(active.as_deref(), Some("documents"));
    });
}

/// The rename field, as assistive technology hears it: its text, and whether it has the
/// keyboard.
fn field(host: &Offscreen) -> Option<(String, bool)> {
    let tree = host.tree();
    let fields = all(&tree, &|node| node["role"] == "text-input");
    let field = fields.first()?;
    let states = field["states"].as_array();
    let focused = states.is_some_and(|states| states.iter().any(|state| state == "focused"));
    Some((text(field, "value"), focused))
}

/// `inline_renaming`'s Icons runs: F2 opens the editor on the cursor's tile with the
/// keyboard in it, and a window key the field passes on leaves it there.
#[test]
fn f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there() {
    hosted_in(
        "icons",
        "f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there",
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            press(&mut host, Key::End, Mods::default());

            press(&mut host, Key::Function(2), Mods::default());
            let editing = host.run_until(PATIENCE, |host| {
                field(host).is_some_and(|(_, focused)| focused)
            });
            assert!(editing, "the field: {:?}", field(&host));

            press(&mut host, Key::Function(5), Mods::default());
            host.run_until(std::time::Duration::from_millis(500), |_| false);
            assert_eq!(field(&host).map(|(_, focused)| focused), Some(true));
        },
    );
}
