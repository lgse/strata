// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use super::{panel_pressed, viewer::Viewer};
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted, send_key},
};

const MODULE: &str = "preview::tests";

fn listed(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = fixture();
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}

fn disable_auto_preview(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, _: &mut Tx| {
        shell::saved(app).change(|settings| settings.single_click_previews = false);
        let prefs = shell::preferences(app);
        app.columns.refresh_mirror_preferences(prefs);
    });
    while host.turn() {}
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn space(host: &mut Offscreen) {
    press(host, Key::Text(" ".to_owned()), Mods::default());
}

fn focus_view(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let key = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(key, true);
    });
    while host.turn() {}
}

fn switch_view(host: &mut Offscreen, mode: &'static str) {
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        shell::show_view(app, mode, tx);
    });
    while host.turn() {}
}

fn select(host: &mut Offscreen, location: &Location) {
    let location = location.clone();
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        let id = shell::id_of(app, &location);
        let view = shell::shown_view(app).expect("a view");
        (view.rows.select_only)(app, (0, id), tx);
    });
    while host.turn() {}
}

fn wait_for_code_preview(host: &mut Offscreen) {
    let loaded = host.run_until(PATIENCE, |host| {
        matches!(host.app().preview.viewer, Viewer::Code(_))
    });
    assert!(loaded, "the text preview loads");
    while host.turn() {}
}

fn focused_key(host: &mut Offscreen) -> Option<String> {
    host.runtime()
        .elements()
        .into_iter()
        .find(|el| el.focused && !el.exiting)
        .and_then(|el| el.key)
}

fn tree_has_name(node: &serde_json::Value, wanted: &str) -> bool {
    if node["name"].as_str() == Some(wanted) {
        return true;
    }
    node["children"]
        .as_array()
        .is_some_and(|children| children.iter().any(|child| tree_has_name(child, wanted)))
}

#[test]
fn space_toggles_file_preview_and_escape_dismisses_keeping_columns_reservation() {
    hosted(
        MODULE,
        "space_toggles_file_preview_and_escape_dismisses_keeping_columns_reservation",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            assert!(
                panel_pressed(host.app()),
                "Columns reserves the slot at launch"
            );

            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            assert!(host.app().preview.enabled);
            wait_for_code_preview(&mut host);
            assert!(host.app().preview.drawer.is_open());

            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.enabled, "Escape closes the preview");
            assert!(
                panel_pressed(host.app()),
                "Columns keeps its slot reservation"
            );
            assert_eq!(host.app().browser().selected_entries().len(), 1);

            switch_view(&mut host, "list");
            assert!(!panel_pressed(host.app()), "List does not reserve the slot");
            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            assert!(host.app().preview.enabled);
            wait_for_code_preview(&mut host);
            assert!(panel_pressed(host.app()));

            press(&mut host, Key::Escape, Mods::default());
            assert!(!panel_pressed(host.app()));
        },
    );
}

#[test]
fn columns_mirrors_file_preview_by_default_and_context_row_previews() {
    hosted(
        MODULE,
        "columns_mirrors_file_preview_by_default_and_context_row_previews",
        |mut host| {
            let folder = listed(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));

            select(&mut host, &todo);
            wait_for_code_preview(&mut host);
            assert!(host.app().preview.enabled);

            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.enabled);

            disable_auto_preview(&mut host);
            switch_view(&mut host, "list");
            let todo_for_row = todo.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let id = shell::id_of(app, &todo_for_row);
                assert_eq!(run(app, "preview", On::Row((0, id)), tx), Outcome::Ran);
            });
            wait_for_code_preview(&mut host);
            assert!(host.app().preview.enabled);
        },
    );
}

#[test]
fn preview_panel_command_toggles_drawer_and_columns_reservation() {
    hosted(
        MODULE,
        "preview_panel_command_toggles_drawer_and_columns_reservation",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            assert!(panel_pressed(host.app()));

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                assert_eq!(run(app, "preview-panel", On::Focus, tx), Outcome::Ran);
            });
            while host.turn() {}
            assert!(!panel_pressed(host.app()));
            assert!(!host.app().preview.reserve_columns);

            select(&mut host, &todo);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                assert_eq!(run(app, "preview-panel", On::Focus, tx), Outcome::Ran);
            });
            wait_for_code_preview(&mut host);
            assert!(panel_pressed(host.app()));
            assert!(host.app().preview.reserve_columns);
        },
    );
}

#[test]
fn folder_selection_shows_placeholder_in_icons_and_yields_slot_in_columns() {
    hosted(
        MODULE,
        "folder_selection_shows_placeholder_in_icons_and_yields_slot_in_columns",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            let archive = Location::local(folder.path().join("archive"));

            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            assert!(host.app().preview.enabled);
            wait_for_code_preview(&mut host);

            select(&mut host, &archive);
            let cleared = host.run_until(PATIENCE, |host| host.app().preview.current.is_none());
            assert!(cleared, "selecting a folder clears the file preview");
            assert!(
                !host.app().preview.reserves_empty_preview(host.app()),
                "Columns yields the slot to the child column"
            );

            switch_view(&mut host, "icons");
            select(&mut host, &archive);
            while host.turn() {}
            assert!(host.app().preview.reserves_empty_preview(host.app()));
            assert!(tree_has_name(&host.tree(), "No preview for this selection"));
        },
    );
}

#[test]
fn right_enters_preview_in_columns_and_stays_in_icons() {
    hosted(
        MODULE,
        "right_enters_preview_in_columns_and_stays_in_icons",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));

            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            assert!(host.app().preview.enabled);
            wait_for_code_preview(&mut host);

            press(&mut host, Key::Right, Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some("preview-code"));

            space(&mut host);
            assert!(!host.app().preview.enabled);
            assert_eq!(focused_key(&mut host).as_deref(), Some("columns"));

            switch_view(&mut host, "icons");
            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            assert!(host.app().preview.enabled);
            wait_for_code_preview(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some("icons"));
        },
    );
}

#[test]
fn delete_and_rename_wait_while_preview_has_focus() {
    hosted(
        MODULE,
        "delete_and_rename_wait_while_preview_has_focus",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let path = folder.path().join("todo.txt");
            select(&mut host, &Location::local(&path));
            focus_view(&mut host);
            space(&mut host);
            wait_for_code_preview(&mut host);

            press(&mut host, Key::Right, Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some("preview-code"));

            press(&mut host, Key::Function(2), Mods::default());
            assert!(
                !host.app().rename.state.is_open(),
                "F2 in preview does not rename"
            );

            press(&mut host, Key::Delete, Mods::default());
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            press(&mut host, Key::Delete, shift);
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            press(&mut host, Key::Text("d".to_owned()), ctrl);
            host.run_until(std::time::Duration::from_millis(250), |_| false);
            assert!(
                path.exists(),
                "Delete / Shift+Delete in preview leaves the file"
            );
            assert!(
                !folder.path().join("todo (1).txt").exists(),
                "Ctrl+D in preview does not duplicate the file"
            );
            assert!(!tree_has_name(&host.tree(), "Permanently delete 1 item?"));
        },
    );
}

#[test]
fn clicking_empty_reserved_slot_beside_columns_clears_selection() {
    hosted(
        MODULE,
        "clicking_empty_reserved_slot_beside_columns_clears_selection",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            select(&mut host, &todo);
            assert_eq!(host.app().browser().selected_entries().len(), 1);

            let slot = host
                .runtime()
                .elements()
                .into_iter()
                .find(|el| el.key.as_deref() == Some("preview-slot"))
                .expect("the reserved preview slot");
            let x = slot.rect[0] + slot.rect[2] - 20.0;
            let y = slot.rect[1] + slot.rect[3] / 2.0;
            host.runtime().pointer_down(x, y);
            host.turn();
            host.runtime().pointer_up(x, y);
            while host.turn() {}

            assert!(
                host.app().browser().selected_entries().is_empty(),
                "plain click in empty reserved slot clears selection"
            );
        },
    );
}
