// SPDX-License-Identifier: MIT

//! What the preview follows when the view moves the keyboard or the target without the
//! browser's cursor moving: a view switch, a folder click, Left into the parent column and a
//! filter closing (found by the `preview` fuzz alphabet's smoke tier).

use rune::{Key, Mods, Tx};
use strata_core::model::Location;

use super::{
    disable_auto_preview, focus_view, listed, press, select, space, switch_view, tree_has_name,
    wait_for_code_preview,
};
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted},
};

const MODULE: &str = "preview::tests::follow";
const PLACEHOLDER: &str = "No preview for this selection";

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

fn type_text(host: &mut Offscreen, text: &str) {
    for ch in text.chars() {
        press(host, Key::Text(ch.to_string()), Mods::default());
    }
}

fn shows_placeholder(host: &Offscreen) -> bool {
    host.app().preview.current.is_none()
        && host.app().preview.reserves_empty_preview(host.app())
        && tree_has_name(&host.tree(), PLACEHOLDER)
}

fn previews(host: &Offscreen, name: &str) -> bool {
    host.app()
        .preview
        .current
        .as_ref()
        .is_some_and(|entry| entry.display_name == name)
}

fn row_key(node: &serde_json::Value, name: &str) -> Option<String> {
    if node["role"] == "list-box-option" && node["name"] == name {
        return node["key"].as_str().map(str::to_owned);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| row_key(child, name))
}

/// Clicks the row named `name`, as a scenario's `click` step does.
fn click_row(host: &mut Offscreen, name: &str) {
    let key = row_key(&host.tree(), name).expect("the row is in the tree");
    let elements = host.runtime().elements();
    let row = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key.as_str()))
        .expect("the row is mounted");
    let [x, y, w, h] = row.visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    while host.turn() {}
}

/// GTK's `view_mode_changed` hides the panel only when it is neither enabled nor reserved; the
/// placeholder of the folder selected at launch, shown in Icons, stays when Columns shows.
#[test]
fn a_view_switch_keeps_the_placeholder_of_an_enabled_preview() {
    hosted(
        MODULE,
        "a_view_switch_keeps_the_placeholder_of_an_enabled_preview",
        |mut host| {
            let _folder = listed(&mut host);
            disable_auto_preview(&mut host);
            assert!(
                host.app()
                    .browser()
                    .focused_entry()
                    .is_some_and(|entry| entry.display_name == "archive"),
                "the launch selects the first folder"
            );

            switch_view(&mut host, "icons");
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                assert_eq!(run(app, "preview-panel", On::Focus, tx), Outcome::Ran);
            });
            while host.turn() {}
            assert!(
                shows_placeholder(&host),
                "Icons shows the folder's placeholder"
            );

            switch_view(&mut host, "columns");
            assert!(shows_placeholder(&host), "Columns keeps it");
        },
    );
}

/// A folder a click opens becomes the focused column, so the preview keeps its slot
/// (`lend_slot_to_child`'s `pointer_opens` in `src/ui/preview/lifecycle.rs`).
#[test]
fn clicking_a_folder_in_columns_keeps_the_slot_for_the_placeholder() {
    hosted(
        MODULE,
        "clicking_a_folder_in_columns_keeps_the_slot_for_the_placeholder",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));

            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            wait_for_code_preview(&mut host);

            click_row(&mut host, "archive");
            let shown = host.run_until(PATIENCE, shows_placeholder);
            assert!(shown, "the opened folder's placeholder");
        },
    );
}

/// Left into the parent column raises `FocusChanged` on GTK (`Browser::focus_parent`): the
/// preview follows the parent's open folder and yields the slot to its column.
#[test]
fn left_into_the_parent_column_follows_its_open_folder() {
    hosted(
        MODULE,
        "left_into_the_parent_column_follows_its_open_folder",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let report = Location::local(folder.path().join("documents/report.md"));

            click_row(&mut host, "documents");
            let opened = host.run_until(PATIENCE, |host| {
                host.app().browser().column_entry_counts(1).is_some()
            });
            assert!(opened, "the documents column");
            while host.turn() {}
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let id = crate::shell::id_of(app, &report);
                let view = crate::shell::shown_view(app).expect("a view");
                (view.rows.select_only)(app, (1, id), tx);
            });
            while host.turn() {}
            focus_view(&mut host);
            space(&mut host);
            let loaded = host.run_until(PATIENCE, |host| previews(host, "report.md"));
            assert!(loaded, "the report's preview");

            press(&mut host, Key::Left, Mods::default());
            let followed = host.run_until(PATIENCE, |host| host.app().preview.current.is_none());
            assert!(followed, "the preview follows the parent's folder");
            assert!(!host.app().preview.reserves_empty_preview(host.app()));
        },
    );
}

/// Closing a filter whose result the preview shows puts the listing's cursor back, and the
/// preview follows it as GTK's does.
#[test]
fn closing_a_filter_previews_the_restored_cursor() {
    hosted(
        MODULE,
        "closing_a_filter_previews_the_restored_cursor",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                crate::shell::saved(app).change(|settings| settings.type_to_search = false);
            });
            let todo = Location::local(folder.path().join("todo.txt"));

            select(&mut host, &todo);
            focus_view(&mut host);
            press(&mut host, Key::Text("f".to_owned()), ctrl());
            type_text(&mut host, "readme");
            let found = host.run_until(PATIENCE, |host| {
                crate::filter::targets::under_cursor(host.app()).is_none()
                    && crate::filter::status(host.app()).is_none()
            });
            assert!(found, "the filter's result");
            host.wait(std::time::Duration::from_millis(300));
            press(&mut host, Key::Down, Mods::default());
            space(&mut host);
            let hit = host.run_until(PATIENCE, |host| previews(host, "readme.md"));
            assert!(hit, "the result's preview");

            press(&mut host, Key::Escape, Mods::default());
            let followed = host.run_until(PATIENCE, |host| previews(host, "todo.txt"));
            assert!(followed, "the restored cursor's preview");
        },
    );
}
