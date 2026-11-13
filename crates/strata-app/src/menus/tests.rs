// SPDX-License-Identifier: MIT

use std::{any::TypeId, path::Path};

use rune::{Tx, el};
use strata_components::{
    context_menu::{Item, MenuModel},
    contract::{EntryId, Rect},
};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::{EntryTarget, FolderTarget, Menu, ROWS, Request, entry, folder};
use crate::{
    App, commands,
    host::offscreen::Offscreen,
    shell,
    slots::{Entry, Place, View},
    test_support::{PATIENCE, fixture, hosted, hosted_over, idle_view},
};

fn file(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/tmp/menus/{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Known(0o644),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn directory(name: &str) -> FileEntry {
    FileEntry {
        kind: EntryKind::Directory,
        ..file(name)
    }
}

fn of(entry: &FileEntry, entries: &[FileEntry], typing: bool) -> EntryTarget<'static> {
    // The targets outlive each test's menu: leaked once, a few bytes.
    EntryTarget {
        entry: Box::leak(Box::new(entry.clone())),
        entries: Box::leak(entries.to_vec().into_boxed_slice()),
        can_trash: None,
        can_delete: None,
        typing,
    }
}

fn one(entry: &FileEntry, typing: bool) -> EntryTarget<'static> {
    of(entry, std::slice::from_ref(entry), typing)
}

fn items(model: &MenuModel) -> impl Iterator<Item = &Item> {
    model
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
}

fn labels(model: &MenuModel) -> Vec<&str> {
    items(model).map(|item| item.label.as_str()).collect()
}

fn hint<'a>(model: &'a MenuModel, label: &str) -> Option<&'a str> {
    let item = items(model).find(|item| item.label == label);
    item.expect("the row").shortcut.as_deref()
}

/// The rows of each section, by label: sections are what the separators divide.
fn sections(model: &MenuModel) -> Vec<Vec<&str>> {
    let shown = model
        .sections
        .iter()
        .filter(|section| !section.items.is_empty());
    shown
        .map(|section| {
            section
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect()
        })
        .collect()
}

/// The order `dialogs_and_menus/entry-menu-named-actions` expects of the GTK menu, with the
/// rows that scenario's subsequence check leaves out.
#[test]
fn a_files_menu_lists_the_gtk_menus_rows_in_its_order_under_the_name_and_path() {
    let todo = file("todo.txt");

    let menu = entry(&one(&todo, true));

    assert_eq!(menu.header, ["todo.txt", "/tmp/menus/todo.txt"]);
    assert_eq!(
        sections(&menu),
        [
            vec!["New Folder", "New File"],
            vec!["New Folder with Selection"],
            vec!["Open", "Open With…", "Quick preview"],
            vec!["Cut", "Copy", "Duplicate", "Rename"],
            vec!["Move to…", "Copy to…"],
            vec!["Compress…"],
            vec!["Customize…", "Copy path", "Copy name", "Properties"],
            vec!["Move to Trash", "Permanently delete"],
        ]
    );
    let delete = items(&menu).find(|item| item.label == "Permanently delete");
    assert!(delete.is_some_and(|item| item.danger));
    assert!(items(&menu).all(|item| item.enabled && item.checked.is_none()));
}

#[test]
fn a_folders_menu_swaps_the_preview_for_the_terminal_the_open_in_rows_and_the_pin() {
    let documents = directory("documents");

    let menu = entry(&one(&documents, true));

    let all = labels(&menu);
    assert_eq!(
        all[3..7],
        ["Open", "Open With…", "Open in Terminal", "Open in…"]
    );
    assert!(!all.contains(&"Quick preview"));
    let pin = all.iter().position(|label| *label == "Pin to sidebar");
    let customize = all.iter().position(|label| *label == "Customize…");
    assert_eq!(pin.map(|pin| pin + 1), customize);
    let open_in = items(&menu).find(|item| item.label == "Open in…");
    let children = open_in
        .and_then(|item| item.submenu.as_ref())
        .expect("a submenu");
    let children: Vec<(&str, Option<&str>)> = children[0]
        .items
        .iter()
        .map(|item| (item.label.as_str(), item.shortcut.as_deref()))
        .collect();
    assert_eq!(
        children,
        [
            ("New Tab", Some("Ctrl+Enter")),
            ("New Window", Some("Shift+Enter"))
        ]
    );
}

#[test]
fn an_archive_and_a_program_get_their_rows_after_the_open_rows() {
    let archive = file("photos.zip");
    let program = FileEntry {
        mode: MetadataValue::Known(0o755),
        ..file("run.sh")
    };

    let unpack = entry(&one(&archive, true));
    let run = entry(&one(&program, true));

    assert_eq!(
        sections(&unpack)[2],
        [
            "Open",
            "Open With…",
            "Quick preview",
            "Extract here",
            "Extract to…"
        ]
    );
    assert_eq!(
        sections(&run)[2],
        ["Open", "Open With…", "Quick preview", "Run"]
    );
}

#[test]
fn several_entries_get_the_selections_menu_under_their_count_and_names() {
    let entries = [file("a.txt"), directory("b"), file("c.txt"), file("d.txt")];

    let menu = entry(&of(&entries[0], &entries, true));

    assert_eq!(menu.header, ["4 items selected", "a.txt, b, c.txt, …"]);
    assert_eq!(
        sections(&menu),
        [
            vec!["New Folder", "New File"],
            vec!["New Folder with Selection"],
            vec!["Open", "Open With…", "Open in…"],
            vec!["Cut", "Copy", "Duplicate"],
            vec!["Move to…", "Copy to…"],
            vec!["Compress…"],
            vec!["Copy paths", "Copy names", "Properties"],
            vec!["Move to Trash", "Permanently delete"],
        ]
    );
}

#[test]
fn a_folder_that_cannot_trash_or_delete_loses_those_rows() {
    let todo = file("todo.txt");
    let target = EntryTarget {
        can_trash: Some(false),
        can_delete: Some(false),
        ..one(&todo, true)
    };

    let menu = entry(&target);

    assert_eq!(sections(&menu).len(), 7);
    assert!(!labels(&menu).contains(&"Move to Trash"));
}

/// The hints `entry-menu-named-actions` reads as each row's description.
#[test]
fn hints_are_the_bindings_and_a_typed_one_shows_only_with_type_to_search_off() {
    let documents = directory("documents");
    let typing = entry(&one(&documents, true));
    let keys = entry(&one(&documents, false));

    for (label, expected) in [
        ("New Folder", Some("Ctrl+Shift+N")),
        ("New File", None),
        ("Open", Some("Enter")),
        ("Open in Terminal", Some("Ctrl+Alt+T")),
        ("Cut", Some("Ctrl+X")),
        ("Copy", Some("Ctrl+C")),
        ("Duplicate", Some("Ctrl+D")),
        ("Rename", Some("F2 / Ctrl+R")),
        ("Move to…", None),
        ("Properties", Some("Alt+Enter")),
        ("Move to Trash", Some("Delete")),
        ("Permanently delete", Some("Shift+Delete")),
        ("Copy path", None),
        ("Pin to sidebar", None),
    ] {
        assert_eq!(hint(&typing, label), expected, "{label}");
    }
    assert_eq!(hint(&keys, "Copy path"), Some("Y"));
    assert_eq!(hint(&keys, "Pin to sidebar"), Some("P"));
    let todo = file("todo.txt");
    let preview = entry(&one(&todo, true));
    assert_eq!(hint(&preview, "Quick preview"), Some("Space"));
}

/// The order `dialogs_and_menus/pane-menu-directory-actions` expects of the GTK menu.
#[test]
fn a_panes_menu_lists_the_gtk_menus_rows_with_paste_greyed() {
    let menu = folder(&FolderTarget {
        has_entries: true,
        hidden_shown: false,
        typing: true,
    });

    assert!(menu.header.is_empty());
    assert_eq!(
        sections(&menu),
        [
            vec!["New Folder", "New File", "Paste"],
            vec!["Open With…", "Open in Terminal"],
            vec!["Select All", "Refresh", "Show Hidden Files"],
            vec!["Customize…", "Properties"],
        ]
    );
    let greyed: Vec<&str> = items(&menu)
        .filter(|item| !item.enabled)
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(greyed, ["Paste"]);
    assert_eq!(hint(&menu, "Properties"), None);
    assert_eq!(hint(&menu, "Show Hidden Files"), Some("Ctrl+H"));
    assert_eq!(hint(&menu, "Refresh"), Some("F5"));

    let empty = folder(&FolderTarget {
        has_entries: false,
        hidden_shown: true,
        typing: true,
    });
    let select_all = items(&empty).find(|item| item.label == "Select All");
    assert!(select_all.is_some_and(|item| !item.enabled));
    assert!(labels(&empty).contains(&"Hide Hidden Files"));
}

#[test]
fn every_row_is_in_the_catalogs() {
    // Asked of the German catalog by name: the process's language is other tests' too.
    let untranslated: Vec<&str> = ROWS
        .iter()
        .filter(|row| rust_i18n::t!(row.label, locale = "de") == row.label)
        .map(|row| row.label)
        .collect();
    assert_eq!(untranslated, Vec::<&str>::new());
}

/// A row runs the command registered under its id, and is inert while no part has one. The
/// list of rows that act grows as the parts that own the others land.
#[test]
fn the_rows_with_a_command_act_and_the_rest_are_inert() {
    hosted(
        "menus::tests",
        "the_rows_with_a_command_act_and_the_rest_are_inert",
        |host| {
            let mut live: Vec<&str> = ROWS.iter().map(|row| row.id).collect();
            live.dedup();
            live.retain(|id| commands::has(host.app(), id));
            assert_eq!(
                live,
                [
                    "new-folder",
                    "new-file",
                    "open",
                    "rename",
                    "trash",
                    "select-all",
                    "refresh",
                    "hidden"
                ]
            );
        },
    );
}

/// A view the opener has never heard of: it draws one element and answers only what
/// [`Rows`](crate::slots::Rows) asks about a row's position, from the browser's own order.
struct Plain;

fn plain_view() -> View<App> {
    let mut view: View<App> = idle_view(
        "plain",
        |_, _| Some(el().key("plain").w(40.0).h(20.0).focusable(true)),
        |_| true,
    );
    view.rows.position = |app, (depth, id)| {
        let found = app.browser.with_column_entries(depth, |entries| {
            entries
                .iter()
                .position(|entry| shell::id_of(app, &entry.location) == id)
        });
        found.flatten()
    };
    view
}

/// The plain view and the menu, and nothing else.
fn a_plain_view_and_the_menu() -> Vec<Entry<App>> {
    let view = Entry {
        name: "plain",
        kind: TypeId::of::<Plain>,
        place: Place::View(plain_view()),
        listen: |_| None,
        publish: |_| {},
        caught_up: |_| {},
        poll: |_, _, _| false,
        commands: &[],
        keyboard: |_| None,
    };
    vec![view, Entry::of::<Menu>("menu")]
}

/// Opens `folder` in the window of the plain view and waits for the browser to list it.
fn listed(host: &mut Offscreen, folder: &Path, entries: usize) {
    host.app().open(Location::local(folder));
    let counted = |host: &Offscreen| {
        let counts = host.app().browser().column_entry_counts(0);
        counts.is_some_and(|counts| counts.total == entries)
    };
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
}

fn id_of(host: &Offscreen, folder: &Path, name: &str) -> EntryId {
    shell::id_of(host.app(), &Location::local(folder.join(name)))
}

/// Asks for the menu as a view's handler does, and says what opened: the name assistive
/// technology hears, the entry it is for, and the header's lines.
fn opened(host: &mut Offscreen, entry: Option<EntryId>) -> (String, Option<EntryId>, Vec<String>) {
    let request = Request {
        depth: 0,
        entry,
        anchor: Rect::new(10.0, 10.0, 0.0, 0.0),
    };
    host.runtime()
        .update(move |app: &mut App, tx: &mut Tx| super::open(app, request, tx));
    while host.turn() {}
    let menu = &host.app().menu;
    let target = menu.state.target().expect("an open menu").entry();
    let header = menu.state.model().expect("its rows").header.clone();
    (menu.name.clone(), target, header)
}

/// `context_entries` in `src/ui/browser/context_menu.rs`, through a view that gives only
/// its rows' positions.
#[test]
fn a_menu_is_for_the_selection_its_entry_is_in_and_else_for_the_entry_alone() {
    hosted_over(
        "menus::tests",
        "a_menu_is_for_the_selection_its_entry_is_in_and_else_for_the_entry_alone",
        a_plain_view_and_the_menu,
        |mut host| {
            let folder = fixture();
            listed(&mut host, folder.path(), 5);
            let names = ["readme.md".to_owned(), "todo.txt".to_owned()];
            assert!(host.app().browser().select_entries_by_name_at(0, &names));
            while host.turn() {}

            let todo = id_of(&host, folder.path(), "todo.txt");
            let (name, target, header) = opened(&mut host, Some(todo));
            assert_eq!(name, "todo.txt");
            assert_eq!(target, Some(todo));
            assert_eq!(header, ["2 items selected", "readme.md, todo.txt"]);

            let documents = id_of(&host, folder.path(), "documents");
            let (name, target, header) = opened(&mut host, Some(documents));
            assert_eq!(name, "documents");
            assert_eq!(target, Some(documents));
            assert_eq!(header.first().map(String::as_str), Some("documents"));
        },
    );
}

#[test]
fn with_no_entry_the_menu_is_the_folders_and_select_all_needs_something_to_select() {
    hosted_over(
        "menus::tests",
        "with_no_entry_the_menu_is_the_folders_and_select_all_needs_something_to_select",
        a_plain_view_and_the_menu,
        |mut host| {
            let select_all = |host: &Offscreen| {
                let model = host.app().menu.state.model().expect("an open menu");
                let row = items(model).find(|item| item.label == "Select All");
                row.expect("the row").enabled
            };
            let folder = fixture();
            listed(&mut host, folder.path(), 5);

            let (name, target, header) = opened(&mut host, None);
            assert_eq!(name, Location::local(folder.path()).display_name());
            assert_eq!(target, None);
            assert!(header.is_empty());
            assert!(select_all(&host));

            listed(&mut host, &folder.path().join("archive"), 0);
            opened(&mut host, None);
            assert!(!select_all(&host));
        },
    );
}
