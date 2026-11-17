// SPDX-License-Identifier: MIT

use std::path::Path;

use rune::{Key, KeyEvent, Mods};
use strata_components::{
    context_menu::MenuModel,
    contract::{EntryId, Rect},
    progress_dock::jobs::JobStatus,
};
use strata_core::{model::Location, storage::config_directory};
use strata_desktop::keys::{Binding, Key as CatalogKey, Keymap, Map, bindings::*, use_keymap};
use strata_fs::services::InvocationSource;

use super::NumberedActions;
use crate::{
    host::offscreen::Offscreen,
    menus::{self, Request},
    shell,
    test_support::{PATIENCE, folder_of, hosted, send_key},
};

const MODULE: &str = "custom_actions::tests";

fn write_action(id: &str, body: &str) {
    let dir = config_directory().join("actions").join(id);
    std::fs::create_dir_all(&dir).expect("action dir");
    std::fs::write(dir.join("action.toml"), body).expect("action.toml");
}

fn open_folder(host: &mut Offscreen, path: &Path) {
    host.app().open(Location::local(path));
    let listed = |h: &Offscreen| h.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "folder listed");
    while host.turn() {}
    host.runtime().update(|app, tx| {
        if let Some(view) = shell::view_focus(app) {
            tx.focus(view.to_owned(), true);
        }
    });
    while host.turn() {}
}

fn entry_id(host: &Offscreen, folder: &Path, name: &str) -> EntryId {
    shell::id_of(host.app(), &Location::local(folder.join(name)))
}

fn catalog_len(host: &Offscreen) -> usize {
    host.app()
        .custom_actions
        .services
        .as_ref()
        .map_or(0, |s| s.registry.catalog().actions().len())
}

fn section_labels(model: &MenuModel) -> Vec<Vec<String>> {
    model
        .sections
        .iter()
        .filter(|s| !s.items.is_empty())
        .map(|s| s.items.iter().map(|i| i.label.clone()).collect())
        .collect()
}

#[test]
fn menu_places_top_and_submenu_actions_before_edit_rows_and_keeps_them_while_open() {
    hosted(
        MODULE,
        "menu_places_top_and_submenu_actions_before_edit_rows_and_keeps_them_while_open",
        |mut host| {
            write_action(
                "alpha-top",
                "schema_version = 1\nid = \"alpha-top\"\nname = \"Alpha top\"\nmenu = \"top\"\n[when]\nextensions = [\"txt\"]\n[run]\nruntime = \"command\"\nprogram = \"/bin/true\"\nargs = [\"{paths}\"]\n",
            );
            write_action(
                "beta-sub",
                "schema_version = 1\nid = \"beta-sub\"\nname = \"Beta sub\"\nmenu = \"submenu\"\n[when]\n[run]\nruntime = \"command\"\nprogram = \"/bin/true\"\nargs = [\"{paths}\"]\n",
            );
            let folder = folder_of(&["todo.txt"]);
            open_folder(&mut host, folder.path());
            assert!(
                host.run_until(PATIENCE, |h| catalog_len(h) == 2),
                "initial catalog reload"
            );

            let id = entry_id(&host, folder.path(), "todo.txt");
            open_entry_menu(&mut host, id);
            assert_initial_menu_sections(&host);

            // A second submenu action lands while the menu is open: as on GTK, the open menu
            // keeps the rows it opened with, and the next menu shows it.
            write_action(
                "gamma-sub",
                "schema_version = 1\nid = \"gamma-sub\"\nname = \"Gamma sub\"\nmenu = \"submenu\"\n[when]\n[run]\nruntime = \"command\"\nprogram = \"/bin/true\"\nargs = [\"{paths}\"]\n",
            );
            host.runtime().update(|app, _| {
                let _ = super::shared_registry(app);
            });
            assert!(
                host.run_until(PATIENCE, |h| catalog_len(h) == 3),
                "the reload adopts Gamma sub"
            );
            while host.turn() {}
            assert_eq!(
                submenu_item_count(&host),
                Some(1),
                "the open menu is unchanged"
            );

            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(!host.app().menu.state.is_open());
            open_entry_menu(&mut host, id);
            assert_eq!(
                submenu_item_count(&host),
                Some(2),
                "the next menu shows Gamma sub"
            );
        },
    );
}

fn open_entry_menu(host: &mut Offscreen, id: EntryId) {
    host.runtime().update(|app, tx| {
        menus::open(
            app,
            Request {
                depth: 0,
                entry: Some(id),
                anchor: Rect::new(20.0, 20.0, 80.0, 24.0),
            },
            tx,
        );
    });
    while host.turn() {}
}

fn assert_initial_menu_sections(host: &Offscreen) {
    let model = host
        .app()
        .menu
        .state
        .model()
        .expect("open menu model")
        .clone();
    let sections = section_labels(&model);
    let custom_idx = sections
        .iter()
        .position(|s| s.iter().any(|l| l == "Alpha top"))
        .expect("custom section present");
    assert_eq!(sections[custom_idx], ["Alpha top", "Actions"]);
    assert_eq!(sections[custom_idx + 1][0], "Cut");
    let submenu = custom_section(&model).expect("custom section").items[1]
        .submenu
        .as_ref()
        .expect("Actions submenu");
    assert_eq!(submenu[0].items[0].label, "Beta sub");
}

fn custom_section(model: &MenuModel) -> Option<&strata_components::context_menu::Section> {
    model.sections.iter().find(|section| {
        section
            .items
            .iter()
            .any(|item| super::is_menu_action(&item.action))
    })
}

fn submenu_item_count(host: &Offscreen) -> Option<usize> {
    let model = host.app().menu.state.model()?;
    let submenu = custom_section(model)?.items.last()?.submenu.as_ref()?;
    Some(submenu[0].items.len())
}

#[test]
fn confirmed_action_runs_job_opens_dashboard_and_clears_finished_history() {
    hosted(
        MODULE,
        "confirmed_action_runs_job_opens_dashboard_and_clears_finished_history",
        |mut host| {
            let folder = folder_of(&["todo.txt"]);
            let log_path = folder.path().join("recorded.log");
            let manifest = format!(
                "schema_version = 1\nid = \"record\"\nname = \"Record item\"\nmenu = \"top\"\n[when]\nextensions = [\"txt\"]\n[run]\nruntime = \"command\"\nprogram = \"/bin/sh\"\nconfirm = true\nargs = [\"-c\", \"printf '%s\\\\n' \\\"$1\\\" >> '{}'\", \"sh\", \"{{paths}}\"]\n",
                log_path.display()
            );
            write_action("record", &manifest);
            open_folder(&mut host, folder.path());
            assert!(host.run_until(PATIENCE, |h| catalog_len(h) == 1));

            let id = entry_id(&host, folder.path(), "todo.txt");
            open_and_pick_first_custom(&mut host, id);
            assert!(
                host.run_until(PATIENCE, |h| h.app().custom_actions.confirm_open),
                "confirmation dialog opens"
            );

            // Cancel leaves disk untouched and starts no job.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(!host.app().custom_actions.confirm_open);
            assert!(!log_path.exists());

            // Re-open, confirm with Enter on focused Run button, and wait for completion.
            open_and_pick_first_custom(&mut host, id);
            assert!(host.run_until(PATIENCE, |h| focused(&h.tree(), "Run")));
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Enter,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert!(
                host.run_until(PATIENCE, |h| h.app().custom_actions.jobs_state.is_open()),
                "confirming opens the Jobs dashboard"
            );
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app()
                        .custom_actions
                        .job_rows
                        .first()
                        .is_some_and(|r| r.status == JobStatus::Succeeded)
                }),
                "job completes with Succeeded"
            );
            let logged = std::fs::read_to_string(&log_path).expect("recorded.log");
            assert_eq!(
                logged,
                format!("{}\n", folder.path().join("todo.txt").display())
            );

            assert_clear_finished_closes_jobs(&mut host);
        },
    );
}

/// Clear finished with nothing left closes the list and its button, as GTK hides the button,
/// and focus goes back to the view.
fn assert_clear_finished_closes_jobs(host: &mut Offscreen) {
    press_element(host, "jobs-clear");
    assert!(host.app().custom_actions.job_rows.is_empty());
    assert!(!host.app().custom_actions.jobs_state.is_open());
    assert!(
        !mounted(host, "jobs"),
        "the Jobs button goes with the last job"
    );
    let view = shell::view_focus(host.app()).expect("a shown view");
    assert_eq!(focused_key(host).as_deref(), Some(view));
}

fn mounted(host: &Offscreen, key: &str) -> bool {
    host.elements()
        .iter()
        .any(|element| !element.exiting && element.key.as_deref() == Some(key))
}

fn focused_key(host: &Offscreen) -> Option<String> {
    host.elements()
        .into_iter()
        .find(|element| element.focused && !element.exiting)
        .and_then(|element| element.key)
}

fn press_element(host: &mut Offscreen, key: &str) {
    assert!(
        host.run_until(PATIENCE, |h| mounted(h, key)),
        "{key} is mounted"
    );
    let elements = host.elements();
    let element = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key))
        .expect("the element");
    let [x, y, w, h] = element.visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    while host.turn() {}
}

fn focused(node: &serde_json::Value, name: &str) -> bool {
    let has_it = node["name"] == name
        && node["states"]
            .as_array()
            .is_some_and(|states| states.iter().any(|state| state == "focused"));
    has_it
        || node["children"]
            .as_array()
            .is_some_and(|children| children.iter().any(|child| focused(child, name)))
}

fn open_and_pick_first_custom(host: &mut Offscreen, id: EntryId) {
    host.runtime().update(|app, tx| {
        menus::open(
            app,
            Request {
                depth: 0,
                entry: Some(id),
                anchor: Rect::new(20.0, 20.0, 80.0, 24.0),
            },
            tx,
        );
        app.menu.state.close();
        super::activate_menu_action(app, "custom-action:run:0");
    });
    while host.turn() {}
}

#[test]
fn default_map_slots_and_numbered_helpers_report_vacant_mismatch_and_run_matches() {
    hosted(
        MODULE,
        "default_map_slots_and_numbered_helpers_report_vacant_mismatch_and_run_matches",
        |mut host| {
            let folder = folder_of(&["readme.md", "todo.txt"]);
            let ran_path = folder.path().join("slot-ran.txt");
            let manifest = format!(
                "schema_version = 1\nid = \"txt-only\"\nname = \"Txt only\"\nmenu = \"top\"\n[when]\nextensions = [\"txt\"]\n[run]\nruntime = \"command\"\nprogram = \"/bin/sh\"\nargs = [\"-c\", \"printf '%s' \\\"$1\\\" > '{}'\", \"sh\", \"{{paths}}\"]\n",
                ran_path.display()
            );
            write_action("txt-only", &manifest);

            let keymap = Keymap::defaults()
                .with(
                    Map::Default,
                    CUSTOM_ACTION_1,
                    vec![Binding::alt(CatalogKey::_1)],
                )
                .with(
                    Map::Default,
                    CUSTOM_ACTION_2,
                    vec![Binding::alt(CatalogKey::_2)],
                );
            let _keymap = use_keymap(keymap);

            open_folder(&mut host, folder.path());
            assert!(host.run_until(PATIENCE, |h| catalog_len(h) == 1));
            assert_vacant_and_mismatched_slots(&mut host);
            assert_numbered_and_slot_runs(&mut host, &ran_path);
        },
    );
}

fn assert_vacant_and_mismatched_slots(host: &mut Offscreen) {
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    send_key(
        host,
        KeyEvent {
            key: Key::Text("2".to_owned()),
            mods: alt,
        },
    );
    while host.turn() {}
    assert_eq!(host.app().tenxer.flash.text(), Some("No action 2"));

    send_key(
        host,
        KeyEvent {
            key: Key::Text("1".to_owned()),
            mods: alt,
        },
    );
    while host.turn() {}
    assert_eq!(host.app().tenxer.flash.text(), Some("No matching actions"));
}

fn assert_numbered_and_slot_runs(host: &mut Offscreen, ran_path: &Path) {
    send_key(
        host,
        KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        },
    );
    while host.turn() {}
    let mut numbered_opt: Option<NumberedActions> = None;
    host.runtime().update(|app, _| {
        let bg = super::numbered_actions(app, InvocationSource::Background);
        assert!(bg.targets.len() == 1 && bg.actions.is_empty());
        let sel = super::numbered_actions(app, InvocationSource::Selection);
        assert_eq!(sel.actions.len(), 1);
        numbered_opt = Some(sel);
    });
    let sel = numbered_opt.expect("selection numbered actions");
    let first = sel.actions[0].clone();
    host.runtime()
        .update(|app, _| super::run_numbered(app, sel, first));
    assert!(
        host.run_until(PATIENCE, |_| ran_path.exists()),
        "numbered_actions + run_numbered executes on todo.txt"
    );

    std::fs::remove_file(ran_path).expect("clean ran_path");
    send_key(
        host,
        KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        },
    );
    while host.turn() {}
    assert!(!host.app().custom_actions.jobs_state.is_open());
    host.runtime().update(|app, tx| {
        if let Some(view) = shell::view_focus(app) {
            tx.focus(view.to_owned(), true);
        }
    });
    while host.turn() {}
    send_key(
        host,
        KeyEvent {
            key: Key::Text("1".to_owned()),
            mods: Mods {
                alt: true,
                ..Mods::default()
            },
        },
    );
    assert!(
        host.run_until(PATIENCE, |_| ran_path.exists()),
        "Alt+1 runs slot 1 on todo.txt"
    );
}
