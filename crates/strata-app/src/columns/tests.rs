// SPDX-License-Identifier: MIT

//! The adapter on the offscreen host, over real folders. What a test sees of the view it
//! reads from the accessibility tree, as the QA harness will.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::model::Location;

use super::{Load, mirror};
use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, folder_of, send_key},
};

mod history;
mod keys;
mod letters;
mod rename;
mod saved;
mod thumbs;

const MODULE: &str = "columns::tests";

fn hosted(name: &str, run: impl FnOnce(Offscreen)) {
    crate::test_support::hosted(MODULE, name, run);
}

/// One row of a column, as assistive technology hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RowSeen {
    role: String,
    name: String,
    description: String,
    selected: bool,
}

/// One column: its group, the list inside it, and the list's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pane {
    /// Role, name and description.
    group: [String; 3],
    list: [String; 3],
    rows: Vec<RowSeen>,
}

impl Pane {
    fn name(&self) -> &str {
        &self.group[1]
    }

    fn names(&self) -> Vec<&str> {
        self.rows.iter().map(|row| row.name.as_str()).collect()
    }

    fn selected(&self) -> Vec<&str> {
        let selected = self.rows.iter().filter(|row| row.selected);
        selected.map(|row| row.name.as_str()).collect()
    }
}

/// What the tree says of the Columns view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Seen {
    panes: Vec<Pane>,
    /// The name of the strip's active descendant when it is a row: where the keyboard is.
    focused: Option<String>,
    /// The pane the active descendant is in: the folder the browser is working in.
    directory: Option<String>,
}

impl Seen {
    fn pane(&self, name: &str) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.name() == name)
    }

    fn pane_names(&self) -> Vec<&str> {
        self.panes.iter().map(Pane::name).collect()
    }
}

fn text(node: &Value, field: &str) -> String {
    node[field].as_str().unwrap_or_default().to_owned()
}

fn heard(node: &Value) -> [String; 3] {
    ["role", "name", "description"].map(|field| text(node, field))
}

fn children(node: &Value) -> &[Value] {
    node["children"].as_array().map_or(&[], Vec::as_slice)
}

fn descendants<'a>(node: &'a Value, found: &mut Vec<&'a Value>, wanted: &dyn Fn(&Value) -> bool) {
    for child in children(node) {
        if wanted(child) {
            found.push(child);
        } else {
            descendants(child, found, wanted);
        }
    }
}

fn find<'a>(node: &'a Value, wanted: &dyn Fn(&Value) -> bool) -> Vec<&'a Value> {
    let mut found = Vec::new();
    descendants(node, &mut found, wanted);
    found
}

fn has_state(node: &Value, state: &str) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|held| held == state))
}

fn seen(host: &Offscreen) -> Seen {
    let tree = host.tree();
    let is_pane = |node: &Value| node["description"] == "Columns view";
    let panes = find(&tree, &is_pane).into_iter().map(|group| {
        let list = find(group, &|node| node["role"] == "list-box");
        let list = list.first().copied().unwrap_or(&Value::Null);
        let rows = find(list, &|node| node["role"] == "list-box-option");
        Pane {
            group: heard(group),
            list: heard(list),
            rows: rows
                .into_iter()
                .map(|row| RowSeen {
                    role: text(row, "role"),
                    name: text(row, "name"),
                    description: text(row, "description"),
                    selected: has_state(row, "selected"),
                })
                .collect(),
        }
    });
    let strip = find(&tree, &|node| node["activeDescendant"].is_string());
    let active = strip.first().map(|strip| &strip["activeDescendant"]);
    let is_active = |node: &Value| Some(&node["key"]) == active;
    let focused = find(&tree, &is_active)
        .first()
        .filter(|row| row["role"] == "list-box-option")
        .map(|row| text(row, "name"));
    let directory = find(&tree, &is_pane)
        .into_iter()
        .find(|group| !find(group, &is_active).is_empty())
        .map(|group| text(group, "name"));
    Seen {
        panes: panes.collect(),
        focused,
        directory,
    }
}

/// Turns the host until `done`, as a scenario's `wait` step does, and returns what the tree
/// showed then.
fn wait(host: &mut Offscreen, what: &str, done: impl Fn(&Seen) -> bool) -> Seen {
    let reached = host.run_until(PATIENCE, |host| done(&seen(host)));
    let seen = seen(host);
    assert!(reached, "waited for {what}; the tree shows {seen:#?}");
    seen
}

/// Turns the host until neither GLib nor Rune has anything ready, landing each motion at its
/// end as the QA backend will. A timer still running, such as the mirror's wait, does not
/// hold it up.
fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

/// Writes `settings.toml` before the process reads it: for a test's `prepare`.
fn save_settings(contents: &str) {
    let file = settings_file();
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
    std::fs::write(file, contents).expect("settings");
}

fn folder_name(folder: &Path) -> String {
    Location::local(folder).display_name()
}

/// Opens `folder` and waits for its listing.
fn open(host: &mut Offscreen, folder: &Path) -> Seen {
    host.app().open(Location::local(folder));
    let name = folder_name(folder);
    let listed = wait(host, "the folder's listing", |seen| {
        seen.pane(&name).is_some() && host_listed(seen, &name)
    });
    idle(host);
    listed
}

fn host_listed(seen: &Seen, name: &str) -> bool {
    seen.pane(name).is_some_and(|pane| !pane.rows.is_empty())
}

/// A key as a scenario names it: `shift+Down`, `ctrl+a`, `Return`.
fn key_event(chord: &str) -> KeyEvent {
    let mut mods = Mods::default();
    let mut parts: Vec<&str> = chord.split('+').collect();
    let name = parts.pop().expect("a key");
    for modifier in parts {
        match modifier {
            "shift" => mods.shift = true,
            "ctrl" => mods.ctrl = true,
            "alt" => mods.alt = true,
            other => panic!("no modifier {other}"),
        }
    }
    let named = [
        ("Down", Key::Down),
        ("Up", Key::Up),
        ("Left", Key::Left),
        ("Right", Key::Right),
        ("Home", Key::Home),
        ("End", Key::End),
        ("Page_Down", Key::PageDown),
        ("Return", Key::Enter),
        ("Escape", Key::Escape),
        ("Tab", Key::Tab),
        ("BackSpace", Key::Backspace),
        ("F2", Key::Function(2)),
        ("F5", Key::Function(5)),
        ("period", Key::Text(".".to_owned())),
    ];
    let key = named
        .into_iter()
        .find_map(|(known, key)| (known == name).then_some(key))
        .unwrap_or_else(|| {
            assert_eq!(name.chars().count(), 1, "no key {name}");
            Key::Text(name.to_owned())
        });
    KeyEvent { key, mods }
}

/// Feeds the host a key with its modifiers held, as a keyboard sends one.
fn send(host: &mut Offscreen, chord: &str) {
    send_key(host, key_event(chord));
}

fn press(host: &mut Offscreen, chord: &str) {
    send(host, chord);
    idle(host);
}

/// Lets the mirror's wait pass, and what it started settle: what shows then is what stays.
fn rest(host: &mut Offscreen) {
    host.run_until(mirror::WAIT * 3, |_| false);
    idle(host);
}

/// What the browser holds for column `depth`: the names it has selected, and its cursor's.
fn browser_holds(host: &Offscreen, depth: usize) -> (Vec<String>, Option<String>) {
    let browser = host.app().browser();
    let selected = browser.selected_positions(depth);
    let names = browser
        .with_column_entries(depth, |entries| {
            selected
                .iter()
                .map(|position| entries[*position].display_name.clone())
                .collect()
        })
        .unwrap_or_default();
    let cursor = browser.cursor_entry(depth).map(|entry| entry.display_name);
    (names, cursor)
}

/// The browser and the view say the same of every column: which is active, which entries
/// are selected, and where the keyboard is.
fn assert_browser_agrees(host: &Offscreen, after: &str) {
    let seen = seen(host);
    let browser = host.app().browser();
    let focused = browser.focused_entry().map(|entry| entry.display_name);
    assert_eq!(focused, seen.focused, "the focused entry, after {after}");
    let directory = browser
        .active_location()
        .map(|location| location.display_name());
    assert_eq!(
        directory, seen.directory,
        "the active column, after {after}"
    );
    for (depth, pane) in seen.panes.iter().enumerate() {
        let location = browser.location_at(depth).expect("a column");
        assert_eq!(location.display_name(), pane.name(), "after {after}");
        let (selected, _) = browser_holds(host, depth);
        assert_eq!(selected, pane.selected(), "{}, after {after}", pane.name());
    }
    assert_eq!(browser.location_at(seen.panes.len()), None, "after {after}");
}

const FIXTURE: [&str; 5] = ["archive", "documents", "pictures", "readme.md", "todo.txt"];

#[test]
fn a_folder_lists_in_the_browsers_order_without_its_hidden_entries() {
    hosted(
        "a_folder_lists_in_the_browsers_order_without_its_hidden_entries",
        |mut host| {
            let folder = fixture();

            let seen = open(&mut host, folder.path());

            assert_eq!(seen.pane_names(), [folder_name(folder.path())]);
            assert_eq!(seen.panes[0].names(), FIXTURE);
            let held = host.app().browser().with_column_entries(0, |entries| {
                let shown = entries.iter().filter(|entry| !entry.is_hidden);
                shown.map(|entry| entry.display_name.clone()).collect()
            });
            assert_eq!(held, Some(FIXTURE.map(str::to_owned).to_vec()));
            // GTK launches with the first entry focused and selected, which the browser does.
            assert_eq!(seen.focused.as_deref(), Some("archive"));
            assert_eq!(seen.panes[0].selected(), ["archive"]);
            assert_browser_agrees(&host, "the launch");
        },
    );
}

#[test]
fn a_large_folder_arrives_whole_with_each_row_copied_once() {
    hosted(
        "a_large_folder_arrives_whole_with_each_row_copied_once",
        |mut host| {
            const ENTRIES: usize = 10_000;
            let names: Vec<String> = (0..ENTRIES).map(|n| format!("entry-{n:05}")).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let folder = folder_of(&names);
            let copied_before = host.app().rows_copied();

            host.app().open(Location::local(folder.path()));
            let listed = host.run_until(PATIENCE, |host| {
                host.app().columns.load(0) == Some(&Load::Listed)
            });
            idle(&mut host);

            assert!(listed, "the load never finished");
            let app = host.app();
            assert_eq!(app.columns.rows_shown(0), ENTRIES);
            assert_eq!(
                app.browser().with_column_entries(0, <[_]>::len),
                Some(ENTRIES)
            );
            // Every published range once, and no column read again in full.
            assert_eq!(app.rows_copied() - copied_before, ENTRIES as u64);
            // A virtual list mounts the rows near its viewport, as GTK's realizes them.
            let mounted = seen(&host).panes[0].rows.len();
            assert!(
                (1..ENTRIES).contains(&mounted),
                "{mounted} rows are mounted"
            );
        },
    );
}

#[test]
fn the_child_column_follows_the_cursor_after_its_wait_and_not_before() {
    hosted(
        "the_child_column_follows_the_cursor_after_its_wait_and_not_before",
        |mut host| {
            let folder = fixture();
            let root = folder_name(folder.path());
            open(&mut host, folder.path());

            // A pointer parked over the view is reported again whenever what is under it
            // changes. That is not movement, and keys go on mirroring.
            let parked = centre_of(&mut host, &root, "todo.txt");
            host.runtime().pointer_move(parked.0, parked.1);
            idle(&mut host);

            let pressed = Instant::now();
            send(&mut host, "Down");
            // One turn takes the key. The wait runs on GLib's clock, which only a later turn
            // dispatches, so nothing can have opened yet however long this one took.
            host.turn();

            let moved = seen(&host);
            assert_eq!(moved.focused.as_deref(), Some("documents"));
            assert_eq!(moved.pane_names(), [root.as_str()]);
            let mirrored = wait(&mut host, "the child column", |seen| {
                seen.pane("documents")
                    .is_some_and(|pane| pane.rows.len() == 3)
            });
            assert!(pressed.elapsed() >= mirror::WAIT, "{:?}", pressed.elapsed());
            assert_eq!(mirrored.pane_names(), [root.as_str(), "documents"]);
            // Mirroring moves no focus and selects nothing in the child.
            assert_eq!(mirrored.focused.as_deref(), Some("documents"));
            assert!(mirrored.panes[1].selected().is_empty());
            assert_browser_agrees(&host, "the mirror");
            host.runtime().pointer_move(parked.0, parked.1);
            idle(&mut host);

            // Keys that come faster than the wait open nothing on the way: `pictures` is
            // passed over, and the file the cursor stops on closes the child column.
            let copied = host.app().rows_copied();
            send(&mut host, "Down");
            send(&mut host, "Down");
            let on_a_file = wait(&mut host, "the child column to close", |seen| {
                seen.panes.len() == 1
            });
            assert_eq!(on_a_file.focused.as_deref(), Some("readme.md"));
            host.run_until(mirror::WAIT * 3, |_| false);
            assert_eq!(seen(&host).pane_names(), [root.as_str()]);
            assert_eq!(host.app().rows_copied(), copied, "no folder was listed");

            // The pointer moving before the wait is over takes the navigation from the keys.
            send(&mut host, "Up");
            host.turn();
            assert_eq!(seen(&host).focused.as_deref(), Some("pictures"));
            host.runtime().pointer_move(parked.0, parked.1 + 1.0);
            host.run_until(mirror::WAIT * 3, |_| false);
            assert_eq!(seen(&host).pane_names(), [root.as_str()]);
            assert_eq!(host.app().rows_copied(), copied, "no folder was listed");
        },
    );
}

#[test]
fn the_hidden_files_shortcut_shows_and_hides_rows_and_positions_still_map() {
    hosted(
        "the_hidden_files_shortcut_shows_and_hides_rows_and_positions_still_map",
        |mut host| {
            let folder = fixture();
            std::fs::create_dir(folder.path().join(".cache")).expect("a hidden folder");
            let root = folder_name(folder.path());
            open(&mut host, folder.path());

            press(&mut host, "ctrl+period");

            let shown = wait(&mut host, "the hidden entries", |seen| {
                seen.panes[0].rows.len() == 7
            });
            assert_eq!(
                shown.panes[0].names(),
                [
                    ".cache",
                    "archive",
                    "documents",
                    "pictures",
                    ".hidden.txt",
                    "readme.md",
                    "todo.txt"
                ]
            );
            assert!(host.app().browser().preferences().show_hidden);
            // The cursor stays on its entry, which now sits one row down.
            assert_eq!(shown.focused.as_deref(), Some("archive"));
            assert_browser_agrees(&host, "showing hidden entries");

            press(&mut host, "ctrl+h");

            let hidden = wait(&mut host, "the hidden entries to go", |seen| {
                seen.panes[0].rows.len() == 5
            });
            assert_eq!(hidden.panes[0].names(), FIXTURE);
            // A row's place in the view is not its place in the browser, which still holds
            // `.cache` before it: Return must open the folder under the cursor.
            // Return comes before the mirror's wait is over, so it opens the folder itself.
            send(&mut host, "Down");
            send(&mut host, "Return");
            let opened = wait(&mut host, "documents to open", |seen| {
                seen.focused.as_deref() == Some("notes.txt")
            });
            assert_eq!(opened.pane_names(), [root.as_str(), "documents"]);
            assert_browser_agrees(&host, "Return");
        },
    );
}

#[test]
fn a_change_on_disk_reaches_the_listing() {
    hosted("a_change_on_disk_reaches_the_listing", |mut host| {
        let folder = fixture();
        open(&mut host, folder.path());

        std::fs::write(folder.path().join("late.txt"), "late").expect("a new file");
        let created = wait(&mut host, "the new file", |seen| {
            seen.panes[0].names().contains(&"late.txt")
        });
        assert_eq!(
            created.panes[0].names(),
            [
                "archive",
                "documents",
                "pictures",
                "late.txt",
                "readme.md",
                "todo.txt"
            ]
        );

        std::fs::remove_file(folder.path().join("readme.md")).expect("removed");
        let removed = wait(&mut host, "the removed file to go", |seen| {
            !seen.panes[0].names().contains(&"readme.md")
        });
        assert_eq!(
            removed.panes[0].names(),
            ["archive", "documents", "pictures", "late.txt", "todo.txt"]
        );
        // Neither change was the user's: the cursor and the selection stay where they were.
        assert_eq!(removed.focused.as_deref(), Some("archive"));
        assert_browser_agrees(&host, "the changes on disk");

        // Where the cursor goes when its own entry leaves is the browser's to say.
        std::fs::remove_dir(folder.path().join("archive")).expect("removed");
        wait(&mut host, "the next entry to take the cursor", |seen| {
            !seen.panes[0].names().contains(&"archive")
                && seen.focused.as_deref() == Some("documents")
        });
        assert_browser_agrees(&host, "the cursor's entry leaving");
    });
}

#[test]
fn a_folder_that_cannot_be_read_says_why() {
    hosted("a_folder_that_cannot_be_read_says_why", |mut host| {
        let folder = fixture();
        let missing = folder.path().join("never-made");

        host.app().open(Location::local(&missing));
        let failed = host.run_until(PATIENCE, |host| {
            matches!(host.app().columns.load(0), Some(Load::Failed(_)))
        });
        idle(&mut host);

        assert!(failed, "the load never failed");
        let Some(Load::Failed(message)) = host.app().columns.load(0) else {
            unreachable!("the load failed");
        };
        let tree = host.tree();
        let shown = find(&tree, &|node| node["name"] == message.as_str());
        assert_eq!(shown.len(), 1, "{message}");
    });
}

/// The middle of the row named `name` in the pane named `pane`, in window points.
fn centre_of(host: &mut Offscreen, pane: &str, name: &str) -> (f32, f32) {
    let tree = host.tree();
    let is_pane = |node: &Value| node["description"] == "Columns view" && node["name"] == pane;
    let group = find(&tree, &is_pane);
    let row = find(group[0], &|node| {
        node["role"] == "list-box-option" && node["name"] == name
    });
    let key = text(row[0], "key");
    let elements = host.runtime().elements();
    let shown = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key.as_str()));
    let [x, y, w, h] = shown.expect("the row is mounted").visible;
    (x + w / 2.0, y + h / 2.0)
}

/// Clicks the row named `name` in the pane named `pane`, as a scenario's `select` step does.
fn click(host: &mut Offscreen, pane: &str, name: &str) {
    let (x, y) = centre_of(host, pane, name);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

#[test]
fn a_click_selects_in_the_browser_and_opens_or_closes_columns_as_the_gtk_views_do() {
    hosted(
        "a_click_selects_in_the_browser_and_opens_or_closes_columns_as_the_gtk_views_do",
        |mut host| {
            let folder = fixture();
            let root = folder_name(folder.path());
            open(&mut host, folder.path());

            // One click opens a folder in Columns, and the browser works in it.
            click(&mut host, &root, "documents");
            let opened = wait(&mut host, "documents to open", |seen| {
                host_listed(seen, "documents") && seen.directory.as_deref() == Some("documents")
            });
            assert_eq!(opened.panes[0].selected(), ["documents"]);
            assert_browser_agrees(&host, "a click on a folder");

            click(&mut host, "documents", "report.md");
            let selected = wait(&mut host, "the file to be selected", |seen| {
                seen.focused.as_deref() == Some("report.md")
            });
            assert_eq!(selected.panes[1].selected(), ["report.md"]);
            assert_browser_agrees(&host, "a click on a file");

            // A plain click on a file in a column before the last closes the ones after it.
            click(&mut host, &root, "todo.txt");
            let closed = wait(&mut host, "the child column to close", |seen| {
                seen.panes.len() == 1
            });
            assert_eq!(closed.panes[0].selected(), ["todo.txt"]);
            assert_eq!(closed.focused.as_deref(), Some("todo.txt"));
            assert_browser_agrees(&host, "a click on a file in the parent");
        },
    );
}

/// What V0's name probe read from the GTK harness for the same fixture, two Downs in
/// (`.v0/gtk/tree-after.txt` on the spike branch), with Rune's role names: the QA backend
/// maps `group`, `list-box` and `list-box-option` onto AT-SPI's `grouping`, `list` and
/// `list item` (V7).
#[test]
fn panes_lists_and_rows_carry_the_names_the_gtk_views_give_them() {
    hosted(
        "panes_lists_and_rows_carry_the_names_the_gtk_views_give_them",
        |mut host| {
            let folder = fixture();
            std::os::unix::fs::symlink("documents", folder.path().join("shortcut"))
                .expect("a link to a folder");
            std::os::unix::fs::symlink("nowhere", folder.path().join("stale"))
                .expect("a broken link");
            let root = folder_name(folder.path());
            open(&mut host, folder.path());
            press(&mut host, "Down");
            press(&mut host, "Down");

            let seen = wait(&mut host, "the mirrored column", |seen| {
                host_listed(seen, "pictures")
            });

            let named = |role: &str, name: &str, description: &str| {
                [role, name, description].map(str::to_owned)
            };
            let row = |name: &str, description: &str, selected: bool| RowSeen {
                role: "list-box-option".to_owned(),
                name: name.to_owned(),
                description: description.to_owned(),
                selected,
            };
            assert_eq!(
                seen.panes,
                [
                    Pane {
                        group: named("group", &root, "Columns view"),
                        list: named("list-box", &root, "Files"),
                        rows: vec![
                            row("archive", "Folder", false),
                            row("documents", "Folder", false),
                            row("pictures", "Folder", true),
                            row("shortcut", "Folder link", false),
                            row("readme.md", "File", false),
                            row("stale", "Broken link", false),
                            row("todo.txt", "File", false),
                        ],
                    },
                    Pane {
                        group: named("group", "pictures", "Columns view"),
                        list: named("list-box", "pictures", "Files"),
                        rows: vec![
                            row("diagram.txt", "File", false),
                            row("photo.txt", "File", false),
                        ],
                    },
                ]
            );
            assert_eq!(seen.focused.as_deref(), Some("pictures"));
        },
    );
}

/// An empty folder's column is described as GTK's directory surface on a column is then, and
/// draws what it says (`show_empty` in `src/ui/browser/presentation.rs`): the mirrored
/// `archive`, the fixture's empty folder.
#[test]
fn an_empty_column_says_so() {
    hosted("an_empty_column_says_so", |mut host| {
        let folder = fixture();
        open(&mut host, folder.path());
        press(&mut host, "Down");
        press(&mut host, "Up");
        let empty = "This directory is empty";

        let seen = wait(&mut host, "the empty column's status", |seen| {
            seen.panes.get(1).is_some_and(|pane| pane.list[2] == empty)
        });

        assert_eq!(seen.panes[1].list[1], "archive");
        assert!(seen.panes[1].rows.is_empty());
        let tree = host.tree().to_string();
        assert!(tree.matches(empty).count() >= 2, "drawn and heard: {tree}");
    });
}
