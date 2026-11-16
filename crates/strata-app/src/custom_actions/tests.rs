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
fn menu_places_top_and_submenu_actions_before_edit_rows_and_refreshes_open_menu_live() {
    hosted(
        MODULE,
        "menu_places_top_and_submenu_actions_before_edit_rows_and_refreshes_open_menu_live",
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

            // Add a second submenu action while the menu remains open and trigger reload.
            write_action(
                "gamma-sub",
                "schema_version = 1\nid = \"gamma-sub\"\nname = \"Gamma sub\"\nmenu = \"submenu\"\n[when]\n[run]\nruntime = \"command\"\nprogram = \"/bin/true\"\nargs = [\"{paths}\"]\n",
            );
            host.runtime().update(|app, _| {
                let _ = super::shared_registry(app);
            });
            assert!(
                host.run_until(PATIENCE, |h| submenu_item_count(h) == Some(2)),
                "live menu refresh adopts Gamma sub"
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
    let submenu = model.sections[host.app().menu.custom_slot()].items[1]
        .submenu
        .as_ref()
        .expect("Actions submenu");
    assert_eq!(submenu[0].items[0].label, "Beta sub");
}

fn submenu_item_count(host: &Offscreen) -> Option<usize> {
    let slot = host.app().menu.custom_slot();
    let model = host.app().menu.state.model()?;
    let submenu = model.sections[slot].items.last()?.submenu.as_ref()?;
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

            // ClearFinished empties finished job rows.
            host.runtime().update(|app, _| {
                super::jobs::build_footer_jobs(app, &mut Vec::new());
                let service = super::shared_jobs(app);
                service.clear_finished();
                super::jobs::sync_rows(app);
            });
            assert!(host.app().custom_actions.job_rows.is_empty());
        },
    );
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
