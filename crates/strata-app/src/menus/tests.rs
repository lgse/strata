// SPDX-License-Identifier: MIT

use std::{any::TypeId, path::Path};

use rune::{Key, KeyEvent, Mods, Tx, el};
use strata_components::{
    context_menu::{Item, MenuModel},
    contract::{EntryId, Rect},
};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::{EntryTarget, FolderTarget, Hints, Menu, ROWS, Request};
use crate::{
    App, commands,
    host::offscreen::Offscreen,
    shell,
    slots::{Entry, Place, View},
    test_support::{PATIENCE, fixture, hosted, hosted_after, hosted_over, idle_view, send_key},
};

/// An entry's menu, with the key library its hints are labelled through.
fn entry(target: &EntryTarget<'_>) -> MenuModel {
    crate::startup::install_key_library().expect("the key library");
    super::entry(target)
}

/// A pane's menu, likewise.
fn folder(target: &FolderTarget) -> MenuModel {
    crate::startup::install_key_library().expect("the key library");
    super::folder(target)
}

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
        can_create: true,
        can_trash: None,
        can_delete: None,
        typing: Hints::default_map(typing),
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
            vec!["Open With…", "Open in…"],
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

    let search_hit = EntryTarget {
        can_create: false,
        ..one(&todo, true)
    };
    let hit_menu = entry(&search_hit);
    let hit_labels = labels(&hit_menu);
    assert!(!hit_labels.contains(&"New Folder"));
    assert!(!hit_labels.contains(&"New File"));
    assert!(!hit_labels.contains(&"New Folder with Selection"));
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
    assert_eq!(hint(&keys, "Copy path"), Some("y"));
    assert_eq!(hint(&keys, "Pin to sidebar"), Some("p"));
    let todo = file("todo.txt");
    let preview = entry(&one(&todo, true));
    assert_eq!(hint(&preview, "Quick preview"), Some("Space"));
}

#[test]
fn a_saved_keymap_moves_the_hints_with_the_keys() {
    hosted_after(
        "menus::tests",
        "a_saved_keymap_moves_the_hints_with_the_keys",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "[keymap.default]\nrename = [\"<Control>e\"]\n\
                         toggle_hidden_files = [\"<Control>j\"]\n";
            std::fs::write(file, saved).expect("settings");
        },
        |_host| {
            let todo = file("todo.txt");
            let menu = entry(&one(&todo, true));
            let pane = folder(&FolderTarget {
                has_entries: true,
                can_paste: false,
                can_open_terminal: true,
                hidden_shown: false,
                typing: Hints::default_map(true),
            });

            assert_eq!(hint(&menu, "Rename"), Some("Ctrl+E"));
            assert_eq!(hint(&pane, "Show Hidden Files"), Some("Ctrl+J"));
            assert_eq!(hint(&menu, "Copy"), Some("Ctrl+C"));
        },
    );
}

#[test]
fn a_panes_menu_lists_the_gtk_menus_rows_with_paste_greyed() {
    let menu = folder(&FolderTarget {
        has_entries: true,
        can_paste: false,
        can_open_terminal: true,
        hidden_shown: false,
        typing: Hints::default_map(true),
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

    let pasteable = folder(&FolderTarget {
        has_entries: true,
        can_paste: true,
        can_open_terminal: true,
        hidden_shown: false,
        typing: Hints::default_map(true),
    });
    let paste = items(&pasteable).find(|item| item.label == "Paste");
    assert!(paste.is_some_and(|item| item.enabled));

    let empty = folder(&FolderTarget {
        has_entries: false,
        can_paste: false,
        can_open_terminal: false,
        hidden_shown: true,
        typing: Hints::default_map(true),
    });
    let select_all = items(&empty).find(|item| item.label == "Select All");
    assert!(select_all.is_some_and(|item| !item.enabled));
    let terminal = items(&empty).find(|item| item.label == "Open in Terminal");
    assert!(terminal.is_some_and(|item| !item.enabled));
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
                    "group",
                    "open",
                    "open-with",
                    "preview",
                    "run",
                    "terminal",
                    "open-in-tab",
                    "open-in-window",
                    "extract",
                    "extract-to",
                    "cut",
                    "copy",
                    "duplicate",
                    "rename",
                    "move-to",
                    "copy-to",
                    "compress",
                    "customize",
                    "copy-path",
                    "copy-name",
                    "copy-paths",
                    "copy-names",
                    "properties",
                    "trash",
                    "delete",
                    "paste",
                    "select-all",
                    "refresh",
                    "hidden",
                    "properties"
                ]
            );
        },
    );
}

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
        customized: |_| {},
        poll: |_, _, _| false,
        switched: |_, _| {},
        tab_hidden: |_, _| {},
        tab_shown: |_, _| {},
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

/// Finds the centre of the element keyed `wanted`.
fn center_of(host: &mut Offscreen, wanted: &str) -> (f32, f32) {
    while host.turn() {}
    let elements = host.runtime().elements();
    let [x, y, w, h] = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(wanted))
        .expect("element")
        .visible;
    (x + w / 2.0, y + h / 2.0)
}

#[test]
fn opening_a_menu_activates_its_column_and_returns_focus_to_the_view() {
    hosted(
        "menus::tests",
        "opening_a_menu_activates_its_column_and_returns_focus_to_the_view",
        |mut host| {
            let folder = fixture();
            listed(&mut host, folder.path(), 5);
            let documents = Location::local(folder.path().join("documents"));
            let target = documents.clone();
            host.runtime().update(move |app: &mut App, _| {
                let id = shell::id_of(app, &target);
                let view = shell::shown_view(app).expect("a view");
                let position = (view.rows.position)(app, (0, id)).expect("its row");
                app.browser.activate(0, position);
            });
            let second_open = host.run_until(PATIENCE, |host| {
                host.app().browser().location_at(1).as_ref() == Some(&documents)
            });
            assert!(second_open, "documents column");

            host.runtime().update(|_: &mut App, tx: &mut Tx| {
                tx.focus("header-appearance", false);
            });
            while host.turn() {}
            host.runtime().update(|_: &mut App, tx: &mut Tx| {
                assert_eq!(tx.focused(), Some("header-appearance"));
            });
            assert_eq!(host.app().browser().active_depth(), Some(1));

            let pictures = id_of(&host, folder.path(), "pictures");
            let row = strata_components::collection::element::row_key(pictures);
            let (x, y) = center_of(&mut host, &row);
            host.runtime().pointer_move(x, y);
            host.runtime().secondary_down(x, y);
            host.turn();
            host.runtime().secondary_up(x, y);
            while host.turn() {}
            assert!(host.app().menu.state.is_open());
            assert_eq!(host.app().browser().active_depth(), Some(0));

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(!host.app().menu.state.is_open());
            let mut focused = None;
            let mut expected = None;
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                focused = tx.focused().map(str::to_owned);
                expected = shell::view_focus(app).map(str::to_owned);
            });
            assert_eq!(focused, expected);
            assert_eq!(host.app().browser().active_depth(), Some(0));
        },
    );
}

fn current_focus(host: &mut Offscreen) -> Option<String> {
    let mut focused = None;
    host.runtime().update(|_: &mut App, tx: &mut Tx| {
        focused = tx.focused().map(str::to_owned);
    });
    focused
}

#[test]
fn filter_menus_keep_field_focus_on_background_and_focus_view_on_result_row() {
    hosted(
        "menus::tests",
        "filter_menus_keep_field_focus_on_background_and_focus_view_on_result_row",
        |mut host| {
            let folder = fixture();
            listed(&mut host, folder.path(), 5);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                let view = shell::view_focus(app).expect("a view").to_owned();
                tx.focus(view, true);
            });
            while host.turn() {}

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("f".to_owned()),
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                },
            );
            while host.turn() {}
            for ch in "readme".chars() {
                host.runtime().key(KeyEvent {
                    key: Key::Text(ch.to_string()),
                    mods: Mods::default(),
                });
            }
            let hits_shown = host.run_until(PATIENCE, |host| {
                crate::filter::targets::selected_with_depth(host.app()).is_some()
            });
            assert!(hits_shown, "filter results shown");
            let filter_field = current_focus(&mut host).expect("filter field focused");
            assert!(filter_field.starts_with("filter-"), "{filter_field}");

            // Background folder menu (`entry = None`) leaves focus on the filter field.
            opened(&mut host, None);
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(!host.app().menu.state.is_open());
            assert_eq!(
                current_focus(&mut host).as_deref(),
                Some(filter_field.as_str())
            );

            // Result row menu (`entry = Some(hit_id)`) focuses the view (`focus_search_result`).
            let hit_id = id_of(&host, folder.path(), "readme.md");
            let row = strata_components::collection::element::row_key(hit_id);
            let (x, y) = center_of(&mut host, &row);
            host.runtime().pointer_move(x, y);
            host.runtime().secondary_down(x, y);
            host.turn();
            host.runtime().secondary_up(x, y);
            while host.turn() {}
            assert!(host.app().menu.state.is_open());
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(!host.app().menu.state.is_open());
            assert_eq!(
                current_focus(&mut host).as_deref(),
                shell::view_focus(host.app())
            );
        },
    );
}

#[test]
fn tenxer_rows_hint_the_tenxer_verbs() {
    hosted(
        "menus::tests",
        "tenxer_rows_hint_the_tenxer_verbs",
        |mut host| {
            use strata_desktop::keys::{Keys, bindings as key, label};
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
                shell::live_keys(app).follow(shell::preferences(app));
            });
            while host.turn() {}
            let shows = |shown: strata_desktop::keys::Shown| Some(label(&Keys::One(shown)));
            let tenxer = |entry: &FileEntry| EntryTarget {
                typing: Hints::TENXER,
                ..one(entry, false)
            };
            let todo = file("todo.txt");
            let documents = directory("documents");
            let on_file = entry(&tenxer(&todo));
            let on_folder = entry(&tenxer(&documents));
            for (label, expected) in [
                ("Cut", shows(key::YANK_CUT.shown())),
                ("Copy", shows(key::YANK.shown())),
                ("Rename", shows(key::RENAME.alternate(0).shown())),
                ("Move to…", shows(key::MOVE_TO.shown())),
                ("Copy to…", shows(key::COPY_TO.shown())),
                (
                    "Move to Trash",
                    shows(key::MOVE_TO_TRASH.alternate(0).shown()),
                ),
                ("Duplicate", None),
                ("Copy path", None),
            ] {
                assert!(expected.as_ref().is_none_or(|shown| !shown.is_empty()));
                assert_eq!(
                    hint(&on_file, label).map(str::to_owned),
                    expected,
                    "{label}"
                );
            }
            assert_eq!(hint(&on_folder, "Pin to sidebar"), None);
            assert_eq!(hint(&on_folder, "Open in Terminal"), None);
        },
    );
}

#[test]
fn menu_min_width_scales_with_interface_text_size() {
    use strata_kit::{look::Look, shape::LookParams};

    for (text_size, expected) in [(13, 310.0), (26, 620.0), (8, 191.0)] {
        let look = Look::fallback(false).with_params(&LookParams {
            text_size,
            ..LookParams::default()
        });
        assert_eq!(super::min_width(&look), expected, "text_size={text_size}");
    }
}

#[test]
fn every_entry_and_folder_menu_row_carries_its_gtk_icon() {
    use strata_kit::icons::IconName;

    let archive = file("bundle.tar.gz");
    let docs = directory("documents");
    let pair = [file("a.txt"), directory("sub")];
    let models = [
        entry(&one(&archive, false)),
        entry(&one(&docs, false)),
        entry(&of(&pair[0], &pair, false)),
        folder(&FolderTarget {
            has_entries: true,
            can_paste: true,
            can_open_terminal: true,
            hidden_shown: false,
            typing: Hints::default_map(false),
        }),
    ];
    for menu in &models {
        for item in items(menu) {
            assert!(item.icon.is_some(), "row {:?} missing icon", item.label);
            if let Some(sub) = &item.submenu {
                for child in sub.iter().flat_map(|s| s.items.iter()) {
                    assert!(
                        child.icon.is_some(),
                        "submenu row {:?} missing icon",
                        child.label
                    );
                }
            }
        }
    }

    let file_menu = &models[0];
    let icon_of = |label: &str| {
        items(file_menu)
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("missing row {label}"))
            .icon
    };
    for (label, expected) in [
        ("New Folder", IconName::FolderPlus),
        ("New File", IconName::FilePlus),
        ("New Folder with Selection", IconName::FolderPlus),
        ("Open", IconName::Open),
        ("Open With…", IconName::AppWindow),
        ("Quick preview", IconName::Eye),
        ("Extract here", IconName::Extract),
        ("Extract to…", IconName::FolderArchive),
        ("Cut", IconName::Cut),
        ("Copy", IconName::Copy),
        ("Duplicate", IconName::CopyPlus),
        ("Rename", IconName::Rename),
        ("Move to…", IconName::FolderInput),
        ("Copy to…", IconName::FolderOutput),
        ("Compress…", IconName::PackagePlus),
        ("Customize…", IconName::Palette),
        ("Copy path", IconName::Route),
        ("Copy name", IconName::FileType),
        ("Properties", IconName::Properties),
        ("Move to Trash", IconName::Trash),
        ("Permanently delete", IconName::CircleX),
    ] {
        assert_eq!(icon_of(label), Some(expected), "{label}");
    }
    let delete = items(file_menu)
        .find(|item| item.label == "Permanently delete")
        .expect("Permanently delete");
    assert!(delete.danger, "Permanently delete is marked destructive");
}

fn has_element(host: &Offscreen, target: &str) -> bool {
    host.elements()
        .into_iter()
        .any(|el| !el.exiting && el.key.as_deref() == Some(target))
}

fn element_center(host: &mut Offscreen, target: &str) -> (f32, f32) {
    let el = host
        .runtime()
        .elements()
        .into_iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(target))
        .unwrap_or_else(|| panic!("missing element {target}"));
    let [x, y, w, h] = el.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn click_element(host: &mut Offscreen, target: &str) {
    let (x, y) = element_center(host, target);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.runtime().pointer_up(x, y);
    host.turn();
    host.turn();
}

fn right_click_element(host: &mut Offscreen, target: &str) {
    let (x, y) = element_center(host, target);
    host.runtime().pointer_move(x, y);
    host.runtime().secondary_down(x, y);
    host.runtime().secondary_up(x, y);
    host.turn();
    host.turn();
}

fn element_text(host: &mut Offscreen, target: &str) -> String {
    host.runtime()
        .elements()
        .into_iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(target))
        .and_then(|el| el.text)
        .unwrap_or_default()
}

#[test]
fn right_clicking_a_dialog_text_field_opens_the_in_window_field_menu_and_resolves_actions() {
    crate::test_support::in_own_process(
        "menus::tests::right_clicking_a_dialog_text_field_opens_the_in_window_field_menu_and_resolves_actions",
        &[],
        || {
            let ui = crate::startup::UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::with_field_menu(ui, crate::startup::load_settings());
            let board = rune::clipboard::memory();
            host.runtime().use_clipboard(&board);
            host.runtime().update(|app, tx| {
                commands::run(app, "connect-to-server", commands::On::Focus, tx);
            });
            let field_key = |host: &Offscreen| {
                host.elements().into_iter().find_map(|el| {
                    (!el.exiting)
                        .then_some(el.key)
                        .flatten()
                        .filter(|k| k.starts_with("connect-server-") && k.ends_with("-address"))
                })
            };
            assert!(host.run_until(PATIENCE, |h| field_key(h).is_some()));
            let key = field_key(&host).expect("connect-server address field");

            for ch in "smb://nas/docs".chars() {
                host.runtime()
                    .key(KeyEvent::from(Key::Text(ch.to_string())));
            }
            host.turn();
            assert_eq!(element_text(&mut host, &key), "smb://nas/docs");

            right_click_element(&mut host, &key);
            assert!(host.runtime().popup_menu().is_none(), "no native popup");
            assert!(
                host.runtime().find("field-menu-menu").is_some(),
                "in-window field menu opens over modal dialog"
            );
            // Select All (item 3: Cut, Copy, Paste, Select All)
            click_element(&mut host, "field-menu-item3");
            assert!(host.runtime().find("field-menu-menu").is_none());

            // Open again via ContextMenu key (preserving selection) and pick Cut (item 0).
            send_key(&mut host, KeyEvent::from(Key::ContextMenu));
            host.turn();
            host.turn();
            assert!(host.runtime().find("field-menu-menu").is_some());
            click_element(&mut host, "field-menu-item0");
            assert!(
                host.runtime().find("field-menu-menu").is_none(),
                "Cut was enabled and closed the menu"
            );
            assert_eq!(element_text(&mut host, &key), "");
            assert_eq!(
                board.text().as_deref().map(String::as_str),
                Some("smb://nas/docs")
            );

            // Reopen and pick Paste (item 2) to restore the text from clipboard.
            right_click_element(&mut host, &key);
            click_element(&mut host, "field-menu-item2");
            host.turn();
            assert_eq!(element_text(&mut host, &key), "smb://nas/docs");

            // Reopen and dismiss with Escape: focus returns to the text field.
            right_click_element(&mut host, &key);
            assert!(host.runtime().find("field-menu-menu").is_some());
            send_key(&mut host, KeyEvent::from(Key::Escape));
            host.turn();
            host.turn();
            assert!(host.runtime().find("field-menu-menu").is_none());
            assert_eq!(
                host.runtime().presented_focus_key(),
                Some(key.as_str()),
                "Escape returns focus to the text field"
            );
        },
    );
}

#[test]
fn right_clicking_the_location_bar_keeps_editing_open_and_resolves_actions() {
    crate::test_support::in_own_process(
        "menus::tests::right_clicking_the_location_bar_keeps_editing_open_and_resolves_actions",
        &[],
        || {
            let ui = crate::startup::UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::with_field_menu(ui, crate::startup::load_settings());
            let board = rune::clipboard::memory();
            host.runtime().use_clipboard(&board);
            let folder = fixture();
            listed(&mut host, folder.path(), 5);
            assert!(host.run_until(PATIENCE, |h| has_element(h, "location")));

            host.runtime().update(|app, tx| {
                commands::run(app, "location", commands::On::Focus, tx);
            });
            while host.turn() {}
            assert!(host.app().location.state().editing());
            assert!(host.app().location.state().entry_focused());
            let original = host.app().location.state().edit.text.clone();
            assert!(!original.is_empty());

            // Right-clicking the editable location bar opens the field menu without cancelling editing.
            right_click_element(&mut host, "location-field");
            assert!(
                host.runtime().find("field-menu-menu").is_some(),
                "field menu opened on editable location bar"
            );
            assert!(
                host.app().location.state().editing(),
                "opening the field menu did not cancel location bar editing"
            );

            // Select All (item 3) then Cut (item 0) keeps editing open and moves the path to clipboard.
            click_element(&mut host, "field-menu-item3");
            assert!(host.runtime().find("field-menu-menu").is_none());
            assert!(host.app().location.state().editing());

            send_key(&mut host, KeyEvent::from(Key::ContextMenu));
            host.turn();
            host.turn();
            assert!(host.runtime().find("field-menu-menu").is_some());
            assert!(host.app().location.state().editing());
            click_element(&mut host, "field-menu-item0");
            assert!(host.runtime().find("field-menu-menu").is_none());
            assert!(host.app().location.state().editing());
            assert_eq!(host.app().location.state().edit.text, "");
            assert_eq!(
                board.text().as_deref().map(String::as_str),
                Some(original.as_str())
            );

            // Paste (item 2) restores the path and keeps editing open.
            right_click_element(&mut host, "location-field");
            assert!(host.runtime().find("field-menu-menu").is_some());
            click_element(&mut host, "field-menu-item2");
            host.turn();
            assert!(host.app().location.state().editing());
            assert_eq!(host.app().location.state().edit.text, original);

            // Escape closes the field menu and restores focus to the location entry.
            right_click_element(&mut host, "location-field");
            assert!(host.runtime().find("field-menu-menu").is_some());
            send_key(&mut host, KeyEvent::from(Key::Escape));
            host.turn();
            host.turn();
            assert!(host.runtime().find("field-menu-menu").is_none());
            assert!(host.app().location.state().editing());
            assert!(host.app().location.state().entry_focused());
        },
    );
}

#[test]
fn right_clicking_settings_search_and_number_field_opens_field_menu_without_reverting() {
    crate::test_support::in_own_process(
        "menus::tests::right_clicking_settings_search_and_number_field_opens_field_menu_without_reverting",
        &[],
        || {
            let ui = crate::startup::UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::with_field_menu(ui, crate::startup::load_settings());
            host.runtime().update(|app, tx| {
                commands::run(app, "settings", commands::On::Focus, tx);
            });
            assert!(host.run_until(PATIENCE, |h| has_element(h, "settings-search-field")));

            // SettingsSearch has no dummy on_secondary_press, so right-clicking opens the field menu.
            click_element(&mut host, "settings-search-field");
            for ch in "Thumbnail workers".chars() {
                host.runtime()
                    .key(KeyEvent::from(Key::Text(ch.to_string())));
            }
            host.turn();
            right_click_element(&mut host, "settings-search-field");
            assert!(
                host.runtime().find("field-menu-menu").is_some(),
                "SettingsSearch opens the in-window field menu"
            );
            send_key(&mut host, KeyEvent::from(Key::Escape));
            host.turn();
            host.turn();
            assert!(host.runtime().find("field-menu-menu").is_none());

            // NumberField keeps uncommitted text while the field menu is open.
            assert!(host.run_until(PATIENCE, |h| {
                has_element(h, "settings-thumbnail-workers-input")
            }));
            let initial_workers = shell::preferences(host.app()).thumbnail_workers;
            let typed = if initial_workers == 9 { "7" } else { "9" };
            click_element(&mut host, "settings-thumbnail-workers-input");
            host.runtime().key(KeyEvent::from(Key::End));
            host.runtime().key(KeyEvent::from(Key::Backspace));
            host.runtime().key(KeyEvent::from(Key::Backspace));
            host.runtime()
                .key(KeyEvent::from(Key::Text(typed.to_owned())));
            host.turn();
            assert_eq!(
                element_text(&mut host, "settings-thumbnail-workers-input"),
                typed
            );
            assert_eq!(
                shell::preferences(host.app()).thumbnail_workers,
                initial_workers,
                "uncommitted edit has not updated preferences yet"
            );

            right_click_element(&mut host, "settings-thumbnail-workers-input");
            assert!(
                host.runtime().find("field-menu-menu").is_some(),
                "NumberField opens the in-window field menu"
            );
            assert_eq!(
                element_text(&mut host, "settings-thumbnail-workers-input"),
                typed,
                "opening the field menu did not blur-commit or revert NumberField text"
            );
            assert_eq!(
                shell::preferences(host.app()).thumbnail_workers,
                initial_workers
            );

            send_key(&mut host, KeyEvent::from(Key::Escape));
            host.turn();
            host.turn();
            assert!(host.runtime().find("field-menu-menu").is_none());
            assert_eq!(
                element_text(&mut host, "settings-thumbnail-workers-input"),
                typed
            );
        },
    );
}
