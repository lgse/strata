// SPDX-License-Identifier: MIT

//! Inline rename against the adapter, in temporary folders: the editor, the browser's rename
//! behind it, and the file on disk. Each test names the scenario under
//! `tests/qa/scenarios/inline_renaming` whose steps it takes. After a rename the browser, the
//! view and the folder on disk must say the same.

use std::path::Path;

use rune::{Key, KeyEvent, Mods, PhysicalInput};

use super::{
    MODULE, Seen, assert_browser_agrees, click, find, fixture, folder_name, has_state, host_listed,
    hosted, idle, open, press, rest, save_settings, seen, text, wait,
};
use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted_after},
};

/// The rename field, as assistive technology hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Field {
    name: String,
    text: String,
    description: String,
    focused: bool,
    invalid: bool,
    /// The selection's two ends, in the characters Rune counts.
    selection: Option<(u64, u64)>,
}

fn field(host: &Offscreen) -> Option<Field> {
    let tree = host.tree();
    let fields = find(&tree, &|node| node["role"] == "text-input");
    let node = fields.first()?;
    let end = |at: usize| node["selection"][at].as_u64();
    Some(Field {
        name: text(node, "name"),
        text: text(node, "value"),
        description: text(node, "description"),
        focused: has_state(node, "focused"),
        invalid: has_state(node, "invalid"),
        selection: end(1).zip(end(3)),
    })
}

/// Turns the host until the field shows `text` and has the keyboard.
fn wait_for_field(host: &mut Offscreen, text: &str) -> Field {
    let shown =
        |host: &Offscreen| field(host).is_some_and(|field| field.focused && field.text == text);
    let reached = host.run_until(PATIENCE, shown);
    let field = field(host);
    assert!(reached, "waited for the field to show {text:?}: {field:?}");
    idle(host);
    self::field(host).expect("the field")
}

fn wait_for_no_field(host: &mut Offscreen) {
    let reached = host.run_until(PATIENCE, |host| field(host).is_none());
    assert!(reached, "the editor never closed: {:?}", field(host));
    idle(host);
}

/// Types `text` into whatever has the keyboard, a key for each character.
fn type_text(host: &mut Offscreen, text: &str) {
    for character in text.chars() {
        host.runtime().key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
    }
    idle(host);
}

/// Ctrl+A as Rune's window host sends it: a command, with the primary modifier held.
fn select_all(host: &mut Offscreen) {
    let mods = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    let runtime = host.runtime();
    runtime.physical_input(PhysicalInput::Modifiers(mods));
    runtime.key(KeyEvent {
        key: Key::Command('a'),
        mods,
    });
    runtime.physical_input(PhysicalInput::Modifiers(Mods::default()));
    idle(host);
}

fn names_on_disk(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .expect("the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("a name")
        })
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// The view lists what the folder holds, and the browser holds what the view shows.
fn assert_all_agree(host: &Offscreen, folder: &Path, after: &str) {
    let seen = seen(host);
    let mut listed: Vec<String> = seen.panes[0]
        .names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    listed.sort();
    assert_eq!(listed, names_on_disk(folder), "the listing, after {after}");
    assert_browser_agrees(host, after);
}

/// Opens `folder` with the cursor on `name`, as the scenarios' `select_with_keyboard`, and
/// lets the child column it mirrors settle.
fn open_on(host: &mut Offscreen, folder: &Path, name: &str) -> String {
    open(host, folder);
    // Arrived at by a key even where the launch put the cursor there, so the mirror follows.
    press(host, "End");
    for _ in 0..8 {
        if seen(host).focused.as_deref() == Some(name) {
            break;
        }
        press(host, "Up");
    }
    assert_eq!(seen(host).focused.as_deref(), Some(name));
    rest(host);
    folder_name(folder)
}

/// F2 on the cursor's entry, then `typed` over its whole name.
fn retype(host: &mut Offscreen, from: &str, typed: &str) {
    press(host, "F2");
    wait_for_field(host, from);
    select_all(host);
    type_text(host, typed);
    wait_for_field(host, typed);
}

fn read(folder: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(folder.join(name)).ok()
}

/// `rename-commits-on-enter`.
#[test]
fn enter_renames_the_file_and_the_cursor_stays_on_it() {
    hosted(
        "rename::enter_renames_the_file_and_the_cursor_stays_on_it",
        |mut host| {
            let folder = fixture();
            let root = open_on(&mut host, folder.path(), "todo.txt");

            press(&mut host, "F2");
            let opened = wait_for_field(&mut host, "todo.txt");
            // Strata's first selection: the stem, with the caret after it.
            assert_eq!(
                (opened.name.as_str(), opened.selection),
                ("Rename", Some((0, 4)))
            );
            select_all(&mut host);
            type_text(&mut host, "renamed.item");
            wait_for_field(&mut host, "renamed.item");
            press(&mut host, "Return");

            wait_for_no_field(&mut host);
            let renamed = wait(&mut host, "the entry under its new name", |seen: &Seen| {
                let names = seen.panes[0].names();
                names.contains(&"renamed.item") && !names.contains(&"todo.txt")
            });
            assert_eq!(
                read(folder.path(), "renamed.item").as_deref(),
                Some("todo.txt")
            );
            assert_eq!(read(folder.path(), "todo.txt"), None);
            assert_eq!(renamed.focused.as_deref(), Some("renamed.item"));
            let pane = renamed.pane(&root).expect("the pane");
            assert_eq!(pane.selected(), ["renamed.item"]);
            assert_all_agree(&host, folder.path(), "the rename");
        },
    );
}

/// `invalid-names-retain-original/columns-enter`, `rename-moves-into-path/columns-file-as-folder`
/// and `escape-preserves-name`.
#[test]
fn a_refused_name_keeps_the_field_open_with_stratas_message_and_renames_nothing() {
    hosted(
        "rename::a_refused_name_keeps_the_field_open_with_stratas_message_and_renames_nothing",
        |mut host| {
            let folder = fixture();
            open_on(&mut host, folder.path(), "todo.txt");

            for (typed, message) in [
                ("bad//name", "Enter a name"),
                ("todo/", "An existing file can’t become a folder"),
                ("..", "That name is reserved"),
            ] {
                retype(&mut host, "todo.txt", typed);
                let refused = field(&host).expect("the field stays open");
                assert!(refused.focused && refused.invalid, "{typed}: {refused:?}");
                assert_eq!(refused.description, message, "{typed}");

                press(&mut host, "Return");

                wait_for_no_field(&mut host);
                assert_eq!(seen(&host).focused.as_deref(), Some("todo.txt"), "{typed}");
            }
            // A name the rules accept, and Escape.
            retype(&mut host, "todo.txt", "discarded");
            let accepted = field(&host).expect("the field");
            assert!(
                !accepted.invalid && accepted.description.is_empty(),
                "{accepted:?}"
            );
            press(&mut host, "Escape");

            wait_for_no_field(&mut host);
            rest(&mut host);
            assert_eq!(read(folder.path(), "todo.txt").as_deref(), Some("todo.txt"));
            assert_eq!(seen(&host).panes[0].names(), super::FIXTURE);
            assert_eq!(seen(&host).panes[0].selected(), ["todo.txt"]);
            assert_all_agree(&host, folder.path(), "the refused names and Escape");
        },
    );
}

/// `rename-moves-into-path`, `columns-file` and `columns-parent`, up to their undo.
#[test]
fn a_typed_path_moves_the_item_and_what_took_its_place_is_selected() {
    hosted(
        "rename::a_typed_path_moves_the_item_and_what_took_its_place_is_selected",
        |mut host| {
            let folder = fixture();
            let root = open_on(&mut host, folder.path(), "todo.txt");

            retype(&mut host, "todo.txt", "notes/todo.txt");
            press(&mut host, "Return");

            wait_for_no_field(&mut host);
            let placed = wait(
                &mut host,
                "the new folder to take the row",
                |seen: &Seen| {
                    seen.focused.as_deref() == Some("notes")
                        && seen
                            .pane(&root)
                            .is_some_and(|pane| pane.selected() == ["notes"])
                },
            );
            assert!(!placed.panes[0].names().contains(&"todo.txt"));
            assert_eq!(
                read(folder.path(), "notes/todo.txt").as_deref(),
                Some("todo.txt")
            );
            assert_eq!(read(folder.path(), "todo.txt"), None);
            rest(&mut host);
            assert_all_agree(&host, folder.path(), "the move into a new folder");

            // From a folder's own column, `..` takes the item to the folder above.
            click(&mut host, &root, "documents");
            wait(&mut host, "documents to open", |seen: &Seen| {
                host_listed(seen, "documents") && seen.directory.as_deref() == Some("documents")
            });
            click(&mut host, "documents", "notes.txt");
            wait(&mut host, "the file to be selected", |seen: &Seen| {
                seen.focused.as_deref() == Some("notes.txt")
            });
            retype(&mut host, "notes.txt", "../notes.txt");
            press(&mut host, "Return");

            wait_for_no_field(&mut host);
            wait(
                &mut host,
                "the moved file to be selected above",
                |seen: &Seen| {
                    seen.focused.as_deref() == Some("notes.txt")
                        && seen.directory.as_deref() == Some(root.as_str())
                        && seen
                            .pane(&root)
                            .is_some_and(|pane| pane.selected() == ["notes.txt"])
                },
            );
            assert_eq!(
                read(folder.path(), "notes.txt").as_deref(),
                Some("documents/notes.txt")
            );
            assert!(!folder.path().join("documents/notes.txt").exists());
            rest(&mut host);
            assert_all_agree(&host, folder.path(), "the move to the folder above");
        },
    );
}

/// The scenario's own tree: two folders with a file each, an empty folder and two files.
fn tab_tree() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    for (name, inside) in [
        ("delta", Some("in-delta.txt")),
        ("epsilon", Some("in-epsilon.txt")),
        ("notes", None),
    ] {
        std::fs::create_dir(folder.path().join(name)).expect("folder");
        if let Some(inside) = inside {
            std::fs::write(folder.path().join(name).join(inside), name).expect("file");
        }
    }
    for name in ["alpha.txt", "beta.txt"] {
        std::fs::write(folder.path().join(name), name).expect("file");
    }
    folder
}

/// `tab-renames-past-open-folder/columns-mirrored`.
#[test]
fn tab_renames_the_folder_and_carries_the_editor_and_the_child_column_on() {
    hosted(
        "rename::tab_renames_the_folder_and_carries_the_editor_and_the_child_column_on",
        |mut host| {
            let folder = tab_tree();
            let root = open_on(&mut host, folder.path(), "delta");
            wait(&mut host, "the child column of delta", |seen: &Seen| {
                seen.pane_names() == [root.as_str(), "delta"]
            });

            press(&mut host, "F2");
            // A folder's whole name is selected, so typing replaces it.
            let opened = wait_for_field(&mut host, "delta");
            assert_eq!(opened.selection, Some((0, 5)));
            type_text(&mut host, "d2");
            wait_for_field(&mut host, "d2");
            press(&mut host, "Tab");

            // The editor moves on at once, and stays open once the rename lands.
            wait_for_field(&mut host, "epsilon");
            let carried = wait(&mut host, "the rename to land", |seen: &Seen| {
                seen.panes[0].names().contains(&"d2")
                    && seen.pane_names() == [root.as_str(), "epsilon"]
            });
            assert_eq!(
                read(folder.path(), "d2/in-delta.txt").as_deref(),
                Some("delta")
            );
            assert!(!folder.path().join("delta").exists());
            assert_eq!(carried.focused.as_deref(), Some("epsilon"));
            assert_eq!(carried.panes[0].selected(), ["epsilon"]);
            rest(&mut host);
            let held = field(&host).expect("the editor is still open");
            assert!(held.focused && held.text == "epsilon", "{held:?}");
            assert_browser_agrees(&host, "Tab");

            type_text(&mut host, "e2");
            wait_for_field(&mut host, "e2");
            super::send(&mut host, "shift+Tab");
            idle(&mut host);

            wait_for_field(&mut host, "d2");
            wait(&mut host, "the second rename to land", |seen: &Seen| {
                seen.panes[0].names().contains(&"e2") && seen.pane_names() == [root.as_str(), "d2"]
            });
            assert_eq!(
                read(folder.path(), "e2/in-epsilon.txt").as_deref(),
                Some("epsilon")
            );
            rest(&mut host);
            assert!(field(&host).is_some_and(|field| field.focused && field.text == "d2"));
            press(&mut host, "Escape");

            wait_for_no_field(&mut host);
            assert_eq!(
                names_on_disk(folder.path()),
                ["alpha.txt", "beta.txt", "d2", "e2", "notes"]
            );
            assert_eq!(seen(&host).focused.as_deref(), Some("d2"));
            assert_all_agree(&host, folder.path(), "Shift+Tab and Escape");
        },
    );
}

/// `tab-renames-past-open-folder/columns-unmirrored`: with mirroring off the folder's column
/// is opened by a click, which leaves it the keyboard and no cursor, and Left goes back to
/// the folder. The open column then follows its folder's rename.
#[test]
fn an_open_column_follows_its_folders_rename_when_nothing_mirrors() {
    hosted_after(
        MODULE,
        "rename::an_open_column_follows_its_folders_rename_when_nothing_mirrors",
        || save_settings("columns_mirror_selection = false\n"),
        |mut host| {
            let folder = tab_tree();
            let root = folder_name(folder.path());
            open(&mut host, folder.path());
            click(&mut host, &root, "delta");
            let opened = wait(&mut host, "delta to open", |seen: &Seen| {
                host_listed(seen, "delta") && seen.directory.as_deref() == Some("delta")
            });
            assert_eq!(
                opened.focused, None,
                "a click places no cursor in the column"
            );

            press(&mut host, "Left");

            let back = wait(&mut host, "the cursor to rest on delta", |seen: &Seen| {
                seen.focused.as_deref() == Some("delta")
            });
            assert_eq!(back.pane_names(), [root.as_str(), "delta"]);
            assert_browser_agrees(&host, "Left");

            press(&mut host, "F2");
            wait_for_field(&mut host, "delta");
            type_text(&mut host, "d2");
            wait_for_field(&mut host, "d2");
            press(&mut host, "Tab");

            wait_for_field(&mut host, "epsilon");
            let followed = wait(&mut host, "the open column to take its new name", |seen| {
                seen.pane_names() == [root.as_str(), "d2"]
            });
            assert_eq!(followed.panes[1].names(), ["in-delta.txt"]);
            assert_eq!(
                read(folder.path(), "d2/in-delta.txt").as_deref(),
                Some("delta")
            );
            rest(&mut host);
            // Nothing mirrors: the column stays the renamed folder's, with the editor open.
            assert_eq!(seen(&host).pane_names(), [root.as_str(), "d2"]);
            assert!(field(&host).is_some_and(|field| field.focused && field.text == "epsilon"));
            assert_browser_agrees(&host, "Tab");
        },
    );
}

/// What `RenameFailed` does: no scenario of the gate fails a rename the field accepted.
#[test]
fn a_rename_the_browser_fails_opens_the_editor_again_with_its_message() {
    hosted(
        "rename::a_rename_the_browser_fails_opens_the_editor_again_with_its_message",
        |mut host| {
            let folder = fixture();
            open_on(&mut host, folder.path(), "todo.txt");

            // The name is a good one, and taken.
            retype(&mut host, "todo.txt", "readme.md");
            press(&mut host, "Return");

            let reached = host.run_until(PATIENCE, |host| {
                field(host).is_some_and(|field| field.focused && field.invalid)
            });
            let failed = field(&host);
            assert!(reached, "the editor never came back: {failed:?}");
            let failed = failed.expect("the field");
            assert_eq!(failed.text, "readme.md");
            assert!(!failed.description.is_empty(), "the browser's message");
            assert_eq!(read(folder.path(), "todo.txt").as_deref(), Some("todo.txt"));
            assert_eq!(
                read(folder.path(), "readme.md").as_deref(),
                Some("readme.md")
            );

            // Typing asks Strata's rules again, and a free name goes through.
            select_all(&mut host);
            type_text(&mut host, "free.txt");
            let retried = wait_for_field(&mut host, "free.txt");
            assert!(!retried.invalid, "{retried:?}");
            press(&mut host, "Return");

            wait_for_no_field(&mut host);
            wait(&mut host, "the entry under its new name", |seen: &Seen| {
                seen.focused.as_deref() == Some("free.txt")
            });
            assert_eq!(read(folder.path(), "free.txt").as_deref(), Some("todo.txt"));
            assert_all_agree(&host, folder.path(), "the failed rename and the retry");
        },
    );
}

/// What `ItemRelocated` is for: the renamed entry has another id, and a range that held it
/// keeps its anchor, so the next Shift key goes on from it.
#[test]
fn a_rename_inside_a_range_keeps_the_range_and_its_anchor() {
    hosted(
        "rename::a_rename_inside_a_range_keeps_the_range_and_its_anchor",
        |mut host| {
            let folder = fixture();
            open_on(&mut host, folder.path(), "pictures");
            press(&mut host, "shift+Down");
            press(&mut host, "shift+Down");
            assert_eq!(
                seen(&host).panes[0].selected(),
                ["pictures", "readme.md", "todo.txt"]
            );

            retype(&mut host, "todo.txt", "zz.txt");
            press(&mut host, "Return");

            wait_for_no_field(&mut host);
            let renamed = wait(&mut host, "the entry under its new name", |seen: &Seen| {
                seen.focused.as_deref() == Some("zz.txt")
            });
            assert_eq!(
                renamed.panes[0].selected(),
                ["pictures", "readme.md", "zz.txt"]
            );
            assert_all_agree(&host, folder.path(), "the rename");

            // The anchor is still `pictures`: Shift+Up takes the last entry out of the range.
            press(&mut host, "shift+Up");

            assert_eq!(seen(&host).panes[0].selected(), ["pictures", "readme.md"]);
            assert_browser_agrees(&host, "Shift+Up");
        },
    );
}

/// `window-accelerators-leave-rename/columns-f5`: a key the field passes on is no shortcut
/// while the field has the keyboard.
#[test]
fn the_windows_keys_wait_while_the_field_has_the_keyboard() {
    hosted(
        "rename::the_windows_keys_wait_while_the_field_has_the_keyboard",
        |mut host| {
            let folder = fixture();
            open_on(&mut host, folder.path(), "todo.txt");
            retype(&mut host, "todo.txt", "abc");
            let copied = host.app().rows_copied();

            for key in ["F5", "F2", "ctrl+period", "alt+Up"] {
                press(&mut host, key);
                rest(&mut host);

                let held = field(&host).expect("the editor is still open");
                assert!(held.focused && held.text == "abc", "{key}: {held:?}");
                assert_eq!(host.app().rows_copied(), copied, "{key} read a folder");
                assert_eq!(seen(&host).panes[0].names(), super::FIXTURE, "{key}");
            }
            press(&mut host, "Escape");
            wait_for_no_field(&mut host);

            // With the keyboard back in the listing, F5 reads the folder again.
            press(&mut host, "F5");

            let read_again = host.run_until(PATIENCE, |host| host.app().rows_copied() > copied);
            assert!(read_again, "F5 read nothing");
            idle(&mut host);
            assert_all_agree(&host, folder.path(), "F5");
        },
    );
}
