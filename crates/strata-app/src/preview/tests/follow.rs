// SPDX-License-Identifier: MIT

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

fn shows_placeholder(host: &Offscreen) -> bool {
    host.app().preview.current.is_none()
        && host.app().preview.reserves_empty_preview(host.app())
        && tree_has_name(&host.tree(), PLACEHOLDER)
}

fn previews(host: &Offscreen, location: &Location) -> bool {
    host.app()
        .preview
        .current
        .as_ref()
        .is_some_and(|entry| &entry.location == location)
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

fn row_centre(host: &mut Offscreen, name: &str) -> (f32, f32) {
    let key = row_key(&host.tree(), name).expect("the row is in the tree");
    let elements = host.runtime().elements();
    let row = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key.as_str()))
        .expect("the row is mounted");
    let [x, y, w, h] = row.visible;
    (x + w / 2.0, y + h / 2.0)
}

/// Which button a click presses.
#[derive(Clone, Copy)]
enum Button {
    Primary,
    Secondary,
}

fn click_row(host: &mut Offscreen, name: &str, button: Button) {
    let (x, y) = row_centre(host, name);
    host.runtime().pointer_move(x, y);
    match button {
        Button::Primary => host.runtime().pointer_down(x, y),
        Button::Secondary => host.runtime().secondary_down(x, y),
    }
    host.turn();
    match button {
        Button::Primary => host.runtime().pointer_up(x, y),
        Button::Secondary => host.runtime().secondary_up(x, y),
    }
    while host.turn() {}
}

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

            click_row(&mut host, "archive", Button::Primary);
            let shown = host.run_until(PATIENCE, shows_placeholder);
            assert!(shown, "the opened folder's placeholder");
        },
    );
}

#[test]
fn left_into_the_parent_column_follows_its_open_folder() {
    hosted(
        MODULE,
        "left_into_the_parent_column_follows_its_open_folder",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let report = Location::local(folder.path().join("documents/report.md"));

            click_row(&mut host, "documents", Button::Primary);
            let opened = host.run_until(PATIENCE, |host| {
                host.app().browser().column_entry_counts(1).is_some()
            });
            assert!(opened, "the documents column");
            while host.turn() {}
            let shown = report.clone();
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let id = crate::shell::id_of(app, &shown);
                let view = crate::shell::shown_view(app).expect("a view");
                (view.rows.select_only)(app, (1, id), tx);
            });
            while host.turn() {}
            focus_view(&mut host);
            space(&mut host);
            let loaded = host.run_until(PATIENCE, |host| previews(host, &report));
            assert!(loaded, "the report's preview");

            press(&mut host, Key::Left, Mods::default());
            let followed = host.run_until(PATIENCE, |host| host.app().preview.current.is_none());
            assert!(followed, "the preview follows the parent's folder");
            assert!(!host.app().preview.reserves_empty_preview(host.app()));
        },
    );
}

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
            let readme = Location::local(folder.path().join("readme.md"));

            select(&mut host, &todo);
            focus_view(&mut host);
            press(&mut host, Key::Text("f".to_owned()), ctrl());
            for ch in "readme".chars() {
                press(&mut host, Key::Text(ch.to_string()), Mods::default());
            }
            let found = host.run_until(PATIENCE, |host| {
                crate::filter::targets::under_cursor(host.app()).is_none()
                    && crate::filter::status(host.app()).is_none()
            });
            assert!(found, "the filter's result");
            host.wait(std::time::Duration::from_millis(300));
            press(&mut host, Key::Down, Mods::default());
            space(&mut host);
            let hit = host.run_until(PATIENCE, |host| previews(host, &readme));
            assert!(hit, "the result's preview");

            press(&mut host, Key::Escape, Mods::default());
            let followed = host.run_until(PATIENCE, |host| previews(host, &todo));
            assert!(followed, "the restored cursor's preview");
        },
    );
}

#[test]
fn a_context_menu_on_a_child_column_file_previews_it() {
    hosted(
        MODULE,
        "a_context_menu_on_a_child_column_file_previews_it",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            let documents = Location::local(folder.path().join("documents"));
            let notes = Location::local(folder.path().join("documents/notes.txt"));

            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            wait_for_code_preview(&mut host);
            select(&mut host, &documents);
            let lent = host.run_until(PATIENCE, |host| {
                host.app().preview.current.is_none()
                    && host.app().browser().column_entry_counts(1).is_some()
            });
            assert!(lent, "the documents column has the slot");
            while host.turn() {}

            click_row(&mut host, "notes.txt", Button::Secondary);
            let followed = host.run_until(PATIENCE, |host| previews(host, &notes));
            assert!(followed, "the right-clicked file's preview");
            press(&mut host, Key::Escape, Mods::default());

            // The parent column's file, with the next folder's column open beside it.
            press(&mut host, Key::Left, Mods::default());
            press(&mut host, Key::Down, Mods::default());
            let lent = host.run_until(PATIENCE, |host| {
                host.app().preview.current.is_none()
                    && host
                        .app()
                        .browser()
                        .location_at(1)
                        .is_some_and(|at| at.display_name().ends_with("pictures"))
            });
            assert!(lent, "the pictures column has the slot");
            while host.turn() {}
            click_row(&mut host, "todo.txt", Button::Secondary);
            let followed = host.run_until(PATIENCE, |host| previews(host, &todo));
            assert!(followed, "the parent column's right-clicked file's preview");
        },
    );
}

#[test]
fn hiding_a_tab_releases_its_preview_viewer_and_showing_it_reloads() {
    hosted(
        MODULE,
        "hiding_a_tab_releases_its_preview_viewer_and_showing_it_reloads",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let provider = super::provider::Recorded::default();
            super::provider::install(&mut host, &provider);

            let todo_path = folder.path().join("todo.txt");
            let todo = Location::local(&todo_path);
            select(&mut host, &todo);
            focus_view(&mut host);
            space(&mut host);
            provider.wait_for(&mut host, 0);
            provider.answer_as(
                0,
                "text/plain",
                strata_fs::services::PreviewContent::Text {
                    content: "first load".into(),
                    truncated: false,
                },
            );
            wait_for_code_preview(&mut host);

            let first_tab = crate::shell::tabs::shown_tab(host.app());
            let other_dir = Location::local(folder.path().join("documents"));
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                crate::shell::tabs::open(app, Some(other_dir), 1, tx);
            });
            while host.turn() {}
            assert_eq!(
                provider.evicted(),
                vec![todo_path.clone()],
                "hiding a tab evicts its current preview from the cache"
            );

            let mut viewer_empty_on_return = false;
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                crate::shell::tabs::show(app, first_tab, tx);
                viewer_empty_on_return = app.preview.viewer.is_empty();
            });
            assert!(
                viewer_empty_on_return,
                "parked tab dropped its viewer before being shown again"
            );
            provider.wait_for(&mut host, 1);
            assert_eq!(provider.request(1).entry.location, todo);
            provider.answer_as(
                1,
                "text/plain",
                strata_fs::services::PreviewContent::Text {
                    content: "reloaded".into(),
                    truncated: false,
                },
            );
            wait_for_code_preview(&mut host);
        },
    );
}

#[test]
fn switching_closing_and_resting_evict_preview_caches_without_dropping_visible_viewer() {
    hosted(
        MODULE,
        "switching_closing_and_resting_evict_preview_caches_without_dropping_visible_viewer",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let provider = super::provider::Recorded::default();
            super::provider::install(&mut host, &provider);

            let todo_path = folder.path().join("todo.txt");
            let readme_path = folder.path().join("readme.md");
            select(&mut host, &Location::local(&todo_path));
            focus_view(&mut host);
            space(&mut host);
            provider.wait_for(&mut host, 0);
            provider.answer_as(
                0,
                "text/plain",
                strata_fs::services::PreviewContent::Text {
                    content: "todo".into(),
                    truncated: false,
                },
            );
            wait_for_code_preview(&mut host);

            // Resting clears idle caches without disturbing the visible viewer.
            crate::preview::clear_idle_caches(host.app());
            assert_eq!(provider.clears(), 1);
            assert!(matches!(host.app().preview.viewer, super::Viewer::Code(_)));

            // Switching to another file evicts the outgoing file.
            select(&mut host, &Location::local(&readme_path));
            provider.wait_for(&mut host, 1);
            assert_eq!(provider.evicted(), vec![todo_path.clone()]);
            provider.answer_as(
                1,
                "text/markdown",
                strata_fs::services::PreviewContent::Text {
                    content: "readme".into(),
                    truncated: false,
                },
            );
            wait_for_code_preview(&mut host);

            // Closing the preview evicts the active file.
            press(&mut host, Key::Escape, Mods::default());
            assert_eq!(provider.evicted(), vec![todo_path, readme_path]);
        },
    );
}
