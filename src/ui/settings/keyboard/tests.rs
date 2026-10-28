// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType};

use super::*;
use crate::{test_support::gtk_test, ui::blur::BlurBin};

struct OpenPage {
    window: gtk::Window,
    page: gtk::Widget,
}

impl OpenPage {
    fn open() -> Self {
        let page = keyboard_page(PreferenceManager::shared());
        let window = gtk::Window::new();
        window.set_default_size(1000, 800);
        // Confirmation dialogs layer over the window's overlay, as in Settings.
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&page));
        window.set_child(Some(&overlay));
        window.present();
        settle();
        Self { window, page }
    }

    fn row(&self, map: Map, command: Command) -> gtk::Widget {
        row_in(&self.page, map, command)
    }

    /// A button of the newest confirmation dialog, by its `action-dialog-*`
    /// class; a dismissed dialog lingers while it animates out.
    fn dialog_button(&self, class: &str) -> gtk::Button {
        descendants(self.window.upcast_ref())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
            .filter(|button| button.has_css_class(class))
            .last()
            .unwrap_or_else(|| panic!("{class} button"))
    }

    /// Sends a press through the page's key reader, as the window would.
    fn press(&self, key: Key, modifiers: ModifierType) {
        let content = descendants(&self.page)
            .into_iter()
            .find(|widget| widget.has_css_class("settings-keyboard-page"))
            .expect("page content");
        let controllers = content.observe_controllers();
        for index in 0..controllers.n_items() {
            if let Some(keys) = controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
            {
                keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
            }
        }
        settle();
    }
}

impl Drop for OpenPage {
    fn drop(&mut self) {
        self.window.close();
    }
}

fn settle() {
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        widgets.extend(descendants(&current));
    }
    widgets
}

fn row_in(root: &gtk::Widget, map: Map, command: Command) -> gtk::Widget {
    let table = descendants(root)
        .into_iter()
        .find(|widget| {
            widget.widget_name()
                == match map {
                    Map::Default => "keymap-default",
                    Map::Tenxer => "keymap-tenxer",
                }
        })
        .expect("keymap table");
    descendants(&table)
        .into_iter()
        .find(|widget| widget.widget_name() == format!("settings-search-{}", target_id(command)))
        .expect("command row")
}

fn button(row: &gtk::Widget, label: &str) -> gtk::Button {
    descendants(row)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("{label} button"))
}

fn chips(row: &gtk::Widget) -> Vec<(String, bool)> {
    descendants(row)
        .into_iter()
        .filter(|widget| widget.has_css_class("keymap-chip"))
        .map(|chip| {
            let key = descendants(&chip)
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Label>().ok())
                .expect("chip key");
            (key.text().to_string(), chip.has_css_class("inactive"))
        })
        .collect()
}

fn notes(row: &gtk::Widget) -> Vec<String> {
    descendants(row)
        .into_iter()
        .filter(|widget| widget.has_css_class("keymap-finding") && widget.is_visible())
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

fn capturing(row: &gtk::Widget) -> bool {
    descendants(row)
        .into_iter()
        .any(|widget| widget.has_css_class("keymap-capture") && widget.is_visible())
}

fn rename_entry(map: Map) -> Vec<Binding> {
    PreferenceManager::shared()
        .keymap()
        .bindings(map, RENAME)
        .to_vec()
}

#[test]
fn error_finding_blocks_commit() {
    gtk_test(
        "ui::settings::keyboard::tests::error_finding_blocks_commit",
        || {
            let open = OpenPage::open();
            let row = open.row(Map::Default, RENAME);
            button(&row, "Add").emit_clicked();
            assert!(capturing(&row));
            open.press(Key::Shift_L, ModifierType::empty());
            assert!(capturing(&row), "a modifier alone waits for the key");
            open.press(Key::c, ModifierType::CONTROL_MASK);
            assert_eq!(rename_entry(Map::Default), RENAME.defaults(Map::Default));
            assert!(capturing(&row), "the refused key keeps the capture waiting");
            let refused = notes(&row);
            assert!(
                refused
                    .iter()
                    .any(|note| note.contains("Ctrl+C") && note.contains("“Copy”")),
                "{refused:?}"
            );
            open.press(
                Key::numbersign,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            );
            assert_eq!(rename_entry(Map::Default), RENAME.defaults(Map::Default));
            assert!(capturing(&row), "tab numbers keep Ctrl+Shift+3");
            let refused = notes(&row);
            assert!(
                refused.iter().any(|note| note.contains("Ctrl+Shift+3")),
                "{refused:?}"
            );
            open.press(Key::Escape, ModifierType::empty());
            assert!(!capturing(&row));
            assert!(notes(&row).is_empty(), "{:?}", notes(&row));
            assert_eq!(rename_entry(Map::Default), RENAME.defaults(Map::Default));
        },
    );
}

#[test]
fn warning_saves_and_shows_inactive() {
    gtk_test(
        "ui::settings::keyboard::tests::warning_saves_and_shows_inactive",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_type_to_search(true);
            let open = OpenPage::open();
            let row = open.row(Map::Default, RENAME);
            button(&row, "Add").emit_clicked();
            open.press(Key::e, ModifierType::empty());
            assert!(!capturing(&row));
            assert_eq!(
                rename_entry(Map::Default),
                [
                    Binding::plain(Key::F2),
                    Binding::ctrl(Key::r),
                    Binding::plain(Key::e)
                ]
            );
            assert_eq!(
                chips(&row),
                [
                    ("F2".to_owned(), false),
                    ("Ctrl+R".to_owned(), false),
                    ("e".to_owned(), true)
                ]
            );
            assert!(
                notes(&row)
                    .iter()
                    .any(|note| note.starts_with("e does nothing while type-to-search")),
                "{:?}",
                notes(&row)
            );
            assert_eq!(
                keys::label(&Keys::One(RENAME.shown())),
                "F2 / Ctrl+R",
                "labels leave out a key that never fires"
            );
            button(&row, "Turn off type-to-search").emit_clicked();
            settle();
            assert!(!preferences.type_to_search());
            assert!(chips(&row).iter().all(|(_, inactive)| !inactive));
            assert!(
                descendants(&row)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                    .all(|button| button.label().as_deref() != Some("Turn off type-to-search"))
            );
            assert_eq!(keys::label(&Keys::One(RENAME.shown())), "F2 / Ctrl+R / e");
        },
    );
}

#[test]
fn reset_row_and_reset_table_restore_defaults() {
    gtk_test(
        "ui::settings::keyboard::tests::reset_row_and_reset_table_restore_defaults",
        || {
            let preferences = PreferenceManager::shared();
            let open = OpenPage::open();
            for map in [Map::Default, Map::Tenxer] {
                preferences.set_bindings(map, RENAME, vec![Binding::ctrl(Key::e)]);
                preferences.set_bindings(map, REFRESH, Vec::new());
            }
            settle();
            let reset = |map| button(&open.row(map, RENAME), "Reset");
            // The 10xer table is hidden, so only the button's own visibility counts.
            assert!(reset(Map::Default).get_visible() && reset(Map::Tenxer).get_visible());

            reset(Map::Default).emit_clicked();
            settle();
            assert_eq!(rename_entry(Map::Default), RENAME.defaults(Map::Default));
            assert_eq!(rename_entry(Map::Tenxer), [Binding::ctrl(Key::e)]);
            assert!(!reset(Map::Default).get_visible());

            let reset_all = descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some("Reset all"))
                .expect("Reset all");
            reset_all.emit_clicked();
            settle();
            open.dialog_button("action-dialog-cancel").emit_clicked();
            settle();
            assert!(preferences.keymap().overrides(Map::Default, REFRESH));
            reset_all.emit_clicked();
            settle();
            open.dialog_button("action-dialog-confirm").emit_clicked();
            settle();
            let keymap = preferences.keymap();
            assert!(!keymap.overrides(Map::Default, REFRESH));
            assert!(keymap.overrides(Map::Tenxer, REFRESH));
            assert!(keymap.overrides(Map::Tenxer, RENAME));
            assert!(!reset_all.is_sensitive());

            let tenxer = descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
                .find(|button| button.label().as_deref() == Some("10xer mode"))
                .expect("10xer table choice");
            tenxer.set_active(true);
            settle();
            assert!(reset_all.is_sensitive());
            reset_all.emit_clicked();
            settle();
            open.dialog_button("action-dialog-confirm").emit_clicked();
            settle();
            let keymap = preferences.keymap();
            for map in [Map::Default, Map::Tenxer] {
                assert!(!keymap.overrides(map, RENAME) && !keymap.overrides(map, REFRESH));
            }
        },
    );
}

#[test]
fn page_reflects_external_change() {
    gtk_test(
        "ui::settings::keyboard::tests::page_reflects_external_change",
        || {
            let preferences = PreferenceManager::shared();
            let (first, second) = (OpenPage::open(), OpenPage::open());
            preferences.set_bindings(Map::Default, RENAME, vec![Binding::ctrl(Key::e)]);
            settle();
            for open in [&first, &second] {
                let row = open.row(Map::Default, RENAME);
                assert_eq!(chips(&row), [("Ctrl+E".to_owned(), false)]);
                assert!(button(&row, "Reset").is_visible());
            }
            let copy = second.row(Map::Default, COPY);
            let remove = descendants(&copy)
                .into_iter()
                .find(|widget| widget.has_css_class("keymap-chip-remove"))
                .and_downcast::<gtk::Button>()
                .expect("remove Ctrl+C");
            remove.emit_clicked();
            settle();
            assert!(preferences.keymap().bindings(Map::Default, COPY).is_empty());
            let row = first.row(Map::Default, COPY);
            assert!(chips(&row).is_empty());
            let unbound = descendants(&row)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .find(|label| label.text() == "Not bound")
                .expect("Not bound note");
            assert!(unbound.is_visible());
        },
    );
}

#[test]
fn search_finds_rows_by_action_and_key() {
    gtk_test(
        "ui::settings::keyboard::tests::search_finds_rows_by_action_and_key",
        || {
            crate::ui::prepare_portal_ui();
            let button = gtk::Button::with_label("Settings");
            let root = BlurBin::new(&button);
            let layer = super::super::build_layer(
                &button,
                &root,
                PreferenceManager::shared(),
                Rc::new(|_| {}),
                super::super::install_guard(),
                Rc::new(|_| {}),
            );
            layer.set_visible(true);
            let layer: gtk::Widget = layer.upcast();
            let entry = descendants(&layer)
                .into_iter()
                .find(|widget| widget.has_css_class("settings-global-search-entry"))
                .and_downcast::<gtk::Entry>()
                .expect("global search");
            let stack = descendants(&layer)
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Stack>().ok())
                .expect("settings pages");
            let shown = |widget: &gtk::Widget| {
                let mut current = Some(widget.clone());
                while let Some(widget) = current {
                    if !widget.is_visible() {
                        return false;
                    }
                    if widget
                        .parent()
                        .is_some_and(|parent| parent.is::<gtk::Stack>())
                    {
                        return true;
                    }
                    current = widget.parent();
                }
                false
            };
            for query in ["Rename", "F2"] {
                entry.set_text(query);
                settle();
                assert_eq!(stack.visible_child_name().as_deref(), Some(PAGE), "{query}");
                assert!(shown(&row_in(&layer, Map::Default, RENAME)), "{query}");
                assert!(!shown(&row_in(&layer, Map::Default, COPY)), "{query}");
            }
            entry.set_text("hotkeys");
            settle();
            assert_eq!(stack.visible_child_name().as_deref(), Some(PAGE));
            assert!(shown(&row_in(&layer, Map::Default, RENAME)));
            assert!(shown(&row_in(&layer, Map::Default, COPY)));
            let tenxer = descendants(&stack.child_by_name(PAGE).expect("keyboard page"))
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
                .find(|button| button.label().as_deref() == Some("10xer mode"))
                .expect("10xer table choice");
            assert!(
                shown(tenxer.upcast_ref()) && !tenxer.is_active(),
                "the table switch stays"
            );
            // Only the 10xer table has chord triggers.
            entry.set_text("Sort chord");
            settle();
            assert!(tenxer.is_active());
            assert!(shown(&row_in(&layer, Map::Tenxer, SORT_BY)));
            entry.set_text("");
            settle();
            assert!(shown(&row_in(&layer, Map::Tenxer, COPY)));
        },
    );
}

#[test]
fn filter_shows_rows_whose_action_name_matches() {
    gtk_test(
        "ui::settings::keyboard::tests::filter_shows_rows_whose_action_name_matches",
        || {
            let open = OpenPage::open();
            let filter = descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Entry>().ok())
                .find(|entry| entry.placeholder_text().as_deref() == Some("Search keybinds"))
                .expect("keybind filter");
            let no_match = descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .find(|label| label.text() == "No matching shortcuts")
                .expect("empty note");
            let shown = |map: Map, command: Command| open.row(map, command).is_drawable();

            filter.set_text("RENAME");
            settle();
            assert!(shown(Map::Default, RENAME));
            assert!(!shown(Map::Default, COPY));
            assert!(!no_match.is_visible());

            // Key labels are not names: F2 is Rename's key.
            filter.set_text("F2");
            settle();
            assert!(!shown(Map::Default, RENAME));
            assert!(no_match.is_drawable());

            filter.set_text("rename");
            settle();
            let tenxer = descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
                .find(|button| button.label().as_deref() == Some("10xer mode"))
                .expect("10xer table choice");
            tenxer.set_active(true);
            settle();
            assert!(
                shown(Map::Tenxer, RENAME),
                "the filter follows the shown table"
            );
            assert!(!shown(Map::Tenxer, COPY));

            filter.set_text("");
            settle();
            assert!(shown(Map::Tenxer, RENAME) && shown(Map::Tenxer, COPY));
        },
    );
}

#[test]
fn tables_list_only_what_their_map_can_run() {
    gtk_test(
        "ui::settings::keyboard::tests::tables_list_only_what_their_map_can_run",
        || {
            let listed = |map: Map| sections(map);
            let has = |map: Map, command: Command| {
                listed(map)
                    .iter()
                    .any(|(_, commands)| commands.contains(&command))
            };
            let default = listed(Map::Default);
            assert!(default.iter().all(|(title, _)| *title != "10xer mode"));
            assert!(!has(Map::Default, FIND), "a 10xer verb");
            assert!(!has(Map::Default, YANK), "a 10xer verb");
            assert!(!has(Map::Default, GO_HOME), "a chord step");
            assert!(
                has(Map::Default, SORT_BY_NAME) && has(Map::Default, SEARCH_SUBFOLDERS),
                "the direct stage runs them"
            );
            assert!(!has(Map::Tenxer, FILTER_SLASH), "a default-map key");
            assert!(has(Map::Tenxer, FIND) && has(Map::Tenxer, GO_HOME));
            assert!(has(Map::Tenxer, DUPLICATE), "the direct stage runs it");
        },
    );
}

#[test]
fn captures_in_two_windows_stay_armed() {
    gtk_test(
        "ui::settings::keyboard::tests::captures_in_two_windows_stay_armed",
        || {
            let (first, second) = (OpenPage::open(), OpenPage::open());
            let rows = [&first, &second].map(|open| open.row(Map::Default, RENAME));
            for row in &rows {
                button(row, "Add").emit_clicked();
            }
            assert!(rows.iter().all(capturing));
            first.press(Key::e, ModifierType::CONTROL_MASK);
            assert!(!capturing(&rows[0]));
            assert!(capturing(&rows[1]), "the other window keeps waiting");
            second.press(Key::F7, ModifierType::empty());
            assert!(!capturing(&rows[1]));
            assert_eq!(
                rename_entry(Map::Default),
                [
                    Binding::plain(Key::F2),
                    Binding::ctrl(Key::r),
                    Binding::ctrl(Key::e),
                    Binding::plain(Key::F7)
                ]
            );
        },
    );
}

#[test]
fn step_rows_follow_their_trigger() {
    gtk_test(
        "ui::settings::keyboard::tests::step_rows_follow_their_trigger",
        || {
            let preferences = PreferenceManager::shared();
            let open = OpenPage::open();
            descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
                .find(|button| button.label().as_deref() == Some("10xer mode"))
                .expect("10xer table choice")
                .set_active(true);
            let row = open.row(Map::Tenxer, GO_HOME);
            let prefix = || {
                descendants(&row)
                    .into_iter()
                    .find(|widget| widget.has_css_class("keymap-prefix"))
                    .and_downcast::<gtk::Label>()
                    .filter(|label| label.get_visible())
                    .map(|label| label.text().to_string())
            };
            assert_eq!(prefix().as_deref(), Some("g"));

            preferences.set_bindings(Map::Tenxer, GO, vec![Binding::ctrl(Key::g)]);
            settle();
            assert_eq!(prefix().as_deref(), Some("Ctrl+G"));

            preferences.set_bindings(Map::Tenxer, GO, Vec::new());
            settle();
            assert_eq!(prefix(), None);
            assert_eq!(chips(&row), [("h".to_owned(), true)]);
            assert!(
                notes(&row).iter().any(|note| note.contains("“Go chord”")),
                "{:?}",
                notes(&row)
            );

            preferences.reset_binding(Map::Tenxer, GO);
            preferences.set_bindings(Map::Tenxer, GO_HOME, vec![Binding::plain(Key::_1)]);
            settle();
            assert!(!preferences.tenxer_mode());
            assert!(
                notes(&row)
                    .iter()
                    .any(|note| note.starts_with("g 1\u{2013}9 open pinned folders")),
                "digits read from the row's table, not the active one: {:?}",
                notes(&row)
            );

            preferences.set_bindings(Map::Tenxer, GO, Vec::new());
            settle();
            assert!(
                notes(&row)
                    .iter()
                    .any(|note| note.starts_with("1\u{2013}9 open pinned folders")),
                "{:?}",
                notes(&row)
            );
        },
    );
}

/// Everything each row of both tables shows, hidden table included.
fn rows_text(page: &gtk::Widget) -> Vec<String> {
    let mut rows = Vec::new();
    for map in [Map::Default, Map::Tenxer] {
        for command in sections(map).into_iter().flat_map(|(_, listed)| listed) {
            let row = row_in(page, map, command);
            let shown = |widget: &gtk::Widget| {
                let mut current = Some(widget.clone());
                while let Some(widget) = current.filter(|widget| *widget != row) {
                    if !widget.get_visible() {
                        return false;
                    }
                    current = widget.parent();
                }
                true
            };
            let texts: Vec<String> = descendants(&row)
                .into_iter()
                .filter(shown)
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .map(|label| label.text().to_string())
                .collect();
            rows.push(format!(
                "{map:?} {}: {texts:?} {:?}",
                command.id(),
                chips(&row)
            ));
        }
    }
    rows
}

#[test]
fn rows_read_their_own_table_in_either_mode() {
    gtk_test(
        "ui::settings::keyboard::tests::rows_read_their_own_table_in_either_mode",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_type_to_search(true);
            preferences.set_bindings(
                Map::Default,
                RENAME,
                vec![
                    Binding::plain(Key::F2),
                    Binding::ctrl(Key::t),
                    Binding::plain(Key::e),
                ],
            );
            preferences.set_bindings(Map::Tenxer, GO_HOME, vec![Binding::plain(Key::_1)]);
            preferences.set_bindings(Map::Tenxer, ACTIONS_EXTRACT, vec![Binding::plain(Key::_2)]);
            preferences.set_bindings(Map::Tenxer, RENAME, vec![Binding::ctrl(Key::c)]);
            let shown_in = |tenxer| {
                preferences.set_tenxer_mode(tenxer);
                let open = OpenPage::open();
                let targets: Vec<(String, String)> = search_targets()
                    .into_iter()
                    .map(|target| (target.id, target.aliases))
                    .collect();
                (rows_text(&open.page), targets)
            };
            let (rows, targets) = shown_in(false);
            for (row, text) in [
                ("Tenxer go.home:", "g 1\u{2013}9 open pinned folders"),
                (
                    "Tenxer actions.extract:",
                    "; 1\u{2013}9 / ; 0 run custom actions",
                ),
                ("Tenxer rename:", "“Copy” also uses Ctrl+C"),
                ("Default rename:", "e does nothing while type-to-search"),
                ("Default rename:", "“New tab” takes Ctrl+T first"),
            ] {
                assert!(
                    rows.iter()
                        .any(|shown| shown.starts_with(row) && shown.contains(text)),
                    "{row} {text}"
                );
            }
            assert_eq!((rows, targets), shown_in(true));
        },
    );
}

/// Two rows of one table with the same name could not tell a user which
/// binding each one edits.
#[test]
fn rows_of_one_table_have_distinct_names() {
    let mut shared = Vec::new();
    for map in [Map::Default, Map::Tenxer] {
        let mut seen: Vec<Command> = Vec::new();
        for command in sections(map).into_iter().flat_map(|(_, commands)| commands) {
            if let Some(other) = seen
                .iter()
                .find(|other| title_in(**other, "en") == title_in(command, "en"))
            {
                shared.push(format!("{map:?}: {other:?} and {command:?}"));
            }
            seen.push(command);
        }
    }
    assert!(shared.is_empty(), "{shared:#?}");
}

#[test]
fn every_command_row_has_a_name_in_every_catalog() {
    use rust_i18n::Backend;
    let catalogs = crate::i18n::Catalogs::compiled();
    for command in keys::registry() {
        assert_ne!(name(command), command.id(), "{command:?} needs a name");
        for locale in catalogs.available_locales() {
            assert!(
                catalogs.translate(&locale, name(command)).is_some(),
                "{locale} has no name for {command:?}"
            );
        }
    }
}

fn write_action(id: &str, name: &str) {
    let directory = crate::storage::config_directory().join("actions").join(id);
    std::fs::create_dir_all(&directory).expect("action folder");
    std::fs::write(
        directory.join("action.toml"),
        format!(
            "schema_version = 1\nid = \"{id}\"\nname = \"{name}\"\n\n[when]\nextensions = [\"txt\"]\n\n[run]\nruntime = \"command\"\nprogram = \"/bin/sh\"\nargs = [\"-c\", \"true\", \"sh\", \"{{paths}}\"]\n"
        ),
    )
    .expect("action manifest");
}

fn reload_actions() {
    glib::MainContext::default().block_on(crate::ui::actions::shared().reload());
    settle();
}

fn labels(row: &gtk::Widget) -> Vec<String> {
    descendants(row)
        .into_iter()
        .filter(|widget| widget.is_visible())
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

#[test]
fn custom_action_slots_name_the_action_they_run() {
    gtk_test(
        "ui::settings::keyboard::tests::custom_action_slots_name_the_action_they_run",
        || {
            write_action("b-resize", "Resize images");
            reload_actions();
            let open = OpenPage::open();
            let slots: Vec<Command> = sections(Map::Default)
                .into_iter()
                .filter(|(title, _)| *title == "Custom actions")
                .flat_map(|(_, commands)| commands)
                .collect();
            assert_eq!(slots, keys::CUSTOM_ACTIONS);
            assert!(
                sections(Map::Tenxer)
                    .iter()
                    .flat_map(|(_, commands)| commands)
                    .all(|command| !keys::CUSTOM_ACTIONS.contains(command)),
                "10xer runs custom actions from its Action chord"
            );
            let slot =
                |number: usize| labels(&open.row(Map::Default, keys::CUSTOM_ACTIONS[number - 1]));
            assert_eq!(slot(1)[..2], ["Custom action 1", "Resize images"]);
            assert_eq!(slot(2)[..2], ["Custom action 2", "No action"]);
            assert_eq!(slot(10)[..2], ["Custom action 10", "No action"]);

            write_action("a-thumbs", "Make thumbnails");
            reload_actions();
            assert_eq!(
                slot(1)[1],
                "Make thumbnails",
                "slots follow the listed order"
            );
            assert_eq!(slot(2)[1], "Resize images");

            std::fs::remove_dir_all(crate::storage::config_directory().join("actions/a-thumbs"))
                .expect("remove action");
            write_action("b-resize", "Shrink images");
            reload_actions();
            assert_eq!(slot(1)[1], "Shrink images");
            assert_eq!(slot(2)[1], "No action");

            let filter = descendants(&open.page)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Entry>().ok())
                .find(|entry| entry.placeholder_text().as_deref() == Some("Search keybinds"))
                .expect("keybind filter");
            let shown = |number: usize| {
                open.row(Map::Default, keys::CUSTOM_ACTIONS[number - 1])
                    .is_drawable()
            };
            filter.set_text("shrink");
            settle();
            assert!(shown(1) && !shown(2));
            filter.set_text("custom action 2");
            settle();
            assert!(shown(2) && !shown(1));

            let names = |number: usize| {
                let id = target_id(keys::CUSTOM_ACTIONS[number - 1]);
                search_targets()
                    .into_iter()
                    .find(|target| target.id == id)
                    .map(|target| target.names)
                    .expect("slot target")
            };
            assert!(names(1).contains("shrink images"), "{}", names(1));
            assert!(names(3).contains("custom action 3"), "{}", names(3));
        },
    );
}
