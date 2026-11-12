// SPDX-License-Identifier: MIT

//! The List view on the offscreen host, over real folders, read from the accessibility tree
//! as the QA harness reads it.

use std::path::{Path, PathBuf};

use rune::{Key, KeyEvent, Mods, PhysicalInput};
use serde_json::Value;
use strata_core::model::Location;

use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted_after},
};

const MODULE: &str = "list::tests";
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

/// The List view as assistive technology hears it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    /// The group around the table: its name and description.
    pane: Option<[String; 2]>,
    /// The table: its name and description.
    table: Option<[String; 2]>,
    /// Each entry's row: its name and description.
    rows: Vec<[String; 2]>,
    selected: Vec<String>,
    /// The table's active descendant: where the keyboard is.
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

/// An entry's row, as the collection keys it: the headings are a row too.
fn is_entry(node: &Value) -> bool {
    node["role"] == "row" && node["key"].as_str().is_some_and(|key| key.starts_with("row-"))
}

fn seen(host: &Offscreen) -> Seen {
    let tree = host.tree();
    let pane = all(&tree, &|node| node["description"] == "List view");
    let Some(pane) = pane.first() else {
        return Seen::default();
    };
    let table = all(pane, &|node| node["role"] == "grid");
    let Some(table) = table.first() else {
        return Seen::default();
    };
    let rows = all(table, &is_entry);
    let selected = |node: &Value| {
        let states = node["states"].as_array();
        states.is_some_and(|states| states.iter().any(|state| state == "selected"))
    };
    let active = &table["activeDescendant"];
    Seen {
        pane: Some(heard(pane)),
        table: Some(heard(table)),
        rows: rows.iter().map(|row| heard(row)).collect(),
        selected: rows
            .iter()
            .filter(|row| selected(row))
            .map(|row| text(row, "name"))
            .collect(),
        focused: rows
            .iter()
            .find(|row| row["key"] == *active)
            .map(|row| text(row, "name")),
    }
}

fn names(seen: &Seen) -> Vec<&str> {
    seen.rows.iter().map(|[name, _]| name.as_str()).collect()
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
    let listed = wait(host, "the folder's listing", |seen| !seen.rows.is_empty());
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

/// `explorer` is what the GTK application read as List before the setting was renamed, and
/// the window opens on the List view for it, named as GTK names the pane, the table and the
/// rows (`src/ui/accessibility.rs`).
#[test]
fn a_saved_explorer_mode_opens_the_list_view_named_as_gtk_names_it() {
    hosted_in(
        "explorer",
        "a_saved_explorer_mode_opens_the_list_view_named_as_gtk_names_it",
        |mut host| {
            let folder = fixture();
            let name = Location::local(folder.path()).display_name();

            let seen = open(&mut host, folder.path());

            assert_eq!(seen.pane, Some([name.clone(), "List view".to_owned()]));
            assert_eq!(seen.table, Some([name, "Files".to_owned()]));
            assert_eq!(names(&seen), FIXTURE);
            let described: Vec<&str> = seen.rows.iter().map(|[_, kind]| kind.as_str()).collect();
            assert_eq!(described, ["Folder", "Folder", "Folder", "File", "File"]);
            assert_eq!(seen.focused.as_deref(), Some("archive"));
            assert_browser_agrees(&host, "the launch");
        },
    );
}

/// The keys of `keyboard_navigation`'s List runs (`arrow-keys-move-focus-remaining`,
/// `shift-arrow-extends-selection`): Up and Down move the cursor and the selection, Right
/// stays on the row, and after every key the browser holds what the table shows.
#[test]
fn keys_move_in_the_table_and_the_browser_holds_the_result() {
    hosted_in(
        "list",
        "keys_move_in_the_table_and_the_browser_holds_the_result",
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            press(&mut host, Key::End, Mods::default());
            press(&mut host, Key::Up, Mods::default());

            let steps: [(Key, Mods, &str, &[&str]); 5] = [
                (Key::Down, Mods::default(), "todo.txt", &["todo.txt"]),
                (Key::Up, Mods::default(), "readme.md", &["readme.md"]),
                (Key::Right, Mods::default(), "readme.md", &["readme.md"]),
                (Key::Down, shift(), "todo.txt", &["readme.md", "todo.txt"]),
                (Key::Up, shift(), "readme.md", &["readme.md"]),
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

/// Return on a folder opens it in place: one table, of the folder's entries, and the browser
/// works in it, as GTK's List view does (`activate_focused_in_place`).
#[test]
fn return_opens_a_folder_in_place() {
    hosted_in("list", "return_opens_a_folder_in_place", |mut host| {
        let folder = fixture();
        open(&mut host, folder.path());
        press(&mut host, Key::Down, Mods::default());

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

/// `inline_renaming`'s List runs: F2 opens the editor in the cursor's row with the keyboard
/// in it, and a window key the field passes on leaves it there.
#[test]
fn f2_opens_the_editor_with_the_keyboard_and_f5_leaves_it_there() {
    hosted_in(
        "list",
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
            assert_eq!(field(&host).map(|(text, _)| text).as_deref(), Some("todo.txt"));

            press(&mut host, Key::Function(5), Mods::default());
            host.run_until(std::time::Duration::from_millis(500), |_| false);
            assert_eq!(field(&host).map(|(_, focused)| focused), Some(true));
        },
    );
}
