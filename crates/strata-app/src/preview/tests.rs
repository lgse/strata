// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use super::{load::live_highlight_waiter_count, panel_pressed, show_document_view, viewer::Viewer};
use crate::{
    App,
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted, send_key},
};
use strata_components::preview_drawer::{DocumentView, HostedPreview};

const MODULE: &str = "preview::tests";

mod follow;
mod model;
mod pdf;
mod provider;
mod route;

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

            // In default mode, Space inside the preview body keeps the drawer open; Escape closes
            // it and returns focus to the listing (`tenxer-l-enters-preview`).
            space(&mut host);
            assert!(host.app().preview.enabled);
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.enabled);
            assert_eq!(focused_key(&mut host).as_deref(), Some("columns"));

            // Plain `i` with type-to-search off must not toggle preview outside 10xer mode.
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.type_to_search = false);
            });
            press(&mut host, Key::Text("i".into()), Mods::default());
            assert!(!host.app().preview.enabled);

            // In 10xer mode, `i` opens the preview from the listing and closes it from inside.
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            press(&mut host, Key::Text("i".into()), Mods::default());
            assert!(host.app().preview.enabled);
            wait_for_code_preview(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some("preview-code"));
            press(&mut host, Key::Text("i".into()), Mods::default());
            assert!(!host.app().preview.enabled);
            assert_eq!(focused_key(&mut host).as_deref(), Some("columns"));
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = false);
            });

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

fn has_element_key(host: &mut Offscreen, key: &str) -> bool {
    host.runtime()
        .elements()
        .iter()
        .any(|el| el.key.as_deref() == Some(key))
}

#[test]
fn markdown_and_csv_render_as_document_and_sheet_and_switch_views() {
    hosted(
        MODULE,
        "markdown_and_csv_render_as_document_and_sheet_and_switch_views",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            std::fs::write(
                folder.path().join("notes.md"),
                "# Heading\n\nBody prose.\n\n```rust\nlet x = 1;\n```\n",
            )
            .expect("write md");
            std::fs::write(folder.path().join("table.csv"), "name,value\nalpha,1\n")
                .expect("write csv");
            host.app().open(Location::local(folder.path()));
            let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, counted));
            while host.turn() {}
            disable_auto_preview(&mut host);

            let md = Location::local(folder.path().join("notes.md"));
            select(&mut host, &md);
            focus_view(&mut host);
            space(&mut host);
            let loaded = host.run_until(PATIENCE, |host| {
                matches!(host.app().preview.viewer, Viewer::Document(_))
            });
            assert!(loaded, "Markdown loads into Viewer::Document");
            while host.turn() {}
            assert!(tree_has_name(&host.tree(), "View source"));
            assert!(has_element_key(&mut host, "preview-document"));

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                show_document_view(app, DocumentView::Source, tx);
            });
            while host.turn() {}
            assert!(tree_has_name(&host.tree(), "View rendered"));
            assert!(has_element_key(&mut host, "preview-code"));

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                show_document_view(app, DocumentView::Rendered, tx);
            });
            while host.turn() {}
            assert!(tree_has_name(&host.tree(), "View source"));

            let csv = Location::local(folder.path().join("table.csv"));
            select(&mut host, &csv);
            let sheet_loaded =
                host.run_until(PATIENCE, |host| host.app().preview.viewer.is_sheet());
            assert!(sheet_loaded, "CSV renders in SheetGrid");
            while host.turn() {}
            assert!(has_element_key(&mut host, "preview-sheet"));
            focus_view(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some("preview-sheet"));
        },
    );
}

#[test]
fn dropping_host_releases_highlight_waiters() {
    let before = live_highlight_waiter_count();
    hosted(
        MODULE,
        "dropping_host_releases_highlight_waiters",
        |mut host| {
            let folder = listed(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            select(&mut host, &todo);
            wait_for_code_preview(&mut host);
            assert!(live_highlight_waiter_count() > before);
        },
    );
    assert_eq!(live_highlight_waiter_count(), before);
}

#[test]
fn switching_code_previews_resets_scroll_to_top() {
    hosted(
        MODULE,
        "switching_code_previews_resets_scroll_to_top",
        |mut host| {
            let folder = fixture();
            let mut first = String::from("first top line\n");
            let mut second = String::from("second top line\n");
            for _ in 0..400 {
                first.push_str("padding line\n");
                second.push_str("padding line\n");
            }
            first.push_str("first bottom needle\n");
            second.push_str("second bottom line\n");
            std::fs::write(folder.path().join("a.txt"), first).expect("write a.txt");
            std::fs::write(folder.path().join("b.txt"), second).expect("write b.txt");
            host.app().open(Location::local(folder.path()));
            let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, counted));
            while host.turn() {}

            let a = Location::local(folder.path().join("a.txt"));
            select(&mut host, &a);
            wait_for_code_preview(&mut host);
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                app.preview.drawer.start_find("needle");
                app.preview.viewer.find("needle");
                tx.wake_after(Duration::ZERO);
            });
            while host.turn() {}
            assert!(tree_has_name(&host.tree(), "first bottom needle"));

            let b = Location::local(folder.path().join("b.txt"));
            select(&mut host, &b);
            let loaded_top = host.run_until(PATIENCE, |host| {
                tree_has_name(&host.tree(), "second top line")
            });
            assert!(loaded_top, "new preview starts scrolled to the top");
        },
    );
}

#[test]
fn right_during_active_preview_load_focuses_viewer_when_shown() {
    hosted(
        MODULE,
        "right_during_active_preview_load_focuses_viewer_when_shown",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));

            select(&mut host, &todo);
            focus_view(&mut host);
            // Send Space then Right back-to-back before the async preview provider finishes.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            host.runtime().frame(1.0 / 60.0);
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Right,
                    mods: Mods::default(),
                },
            );
            host.runtime().frame(1.0 / 60.0);

            wait_for_code_preview(&mut host);
            assert_eq!(focused_key(&mut host).as_deref(), Some("preview-code"));
        },
    );
}

#[test]
fn image_preview_decodes_png_and_supports_enter_and_leave_keys() {
    hosted(
        MODULE,
        "image_preview_decodes_png_and_supports_enter_and_leave_keys",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            let img_path = folder.path().join("photo.png");
            let mut png_bytes = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                24,
                12,
                image::Rgb([20, 140, 60]),
            ))
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
            std::fs::write(&img_path, &png_bytes).expect("write png");

            host.app()
                .browser()
                .navigate(Location::local(folder.path()));
            let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, counted));
            while host.turn() {}
            disable_auto_preview(&mut host);

            select(&mut host, &Location::local(&img_path));
            focus_view(&mut host);
            host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                let (_, _, entry) = app.browser.focused_item().expect("selected photo");
                app.preview.current = Some(entry.clone());
                app.preview.current_depth = Some(0);
                app.preview.enabled = true;
                app.preview.drawer.open(&app.look, tx);
                super::load::render_ready(
                    app,
                    strata_fs::services::Preview {
                        request_id: strata_fs::services::PreviewRequestId(1),
                        entry,
                        content_type: "image/png".to_owned(),
                        content: strata_fs::services::PreviewContent::Rasterized { png: png_bytes },
                    },
                    tx,
                );
            });
            while host.turn() {}
            assert!(matches!(
                host.app().preview.viewer,
                super::viewer::Viewer::Image(_)
            ));
            assert!(tree_has_name(&host.tree(), "photo.png"));

            press(&mut host, Key::Right, Mods::default());
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some("preview-image-picture")
            );
            press(&mut host, Key::Left, Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some("columns"));
            assert_blank_preview_click_and_marquee(&mut host);
        },
    );
}

fn assert_blank_preview_click_and_marquee(host: &mut Offscreen) {
    press(host, Key::Right, Mods::default());
    assert_eq!(focused_key(host).as_deref(), Some("preview-image-picture"));
    let slot = host
        .runtime()
        .elements()
        .into_iter()
        .find(|el| el.key.as_deref() == Some("preview-slot"))
        .expect("open preview slot");
    let x = slot.rect[0] + slot.rect[2] - 24.0;
    let y = slot.rect[1] + slot.rect[3] - 24.0;
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    while host.turn() {}
    assert_eq!(focused_key(host).as_deref(), Some("preview-image-picture"));

    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_move(x - 64.0, y);
    while host.turn() {}
    assert_eq!(focused_key(host).as_deref(), Some("columns"));
    host.runtime().pointer_up(x - 64.0, y);
    while host.turn() {}
}

/// 10xer's J and K scroll the open text preview a line from the listing, where the keyboard
/// stays, and from inside it, where `g g` goes to its top (`scroll_open_preview`,
/// `preview_to_top`).
#[test]
fn tenxer_shift_j_and_k_scroll_the_open_preview_from_the_listing() {
    hosted(
        MODULE,
        "tenxer_shift_j_and_k_scroll_the_open_preview_from_the_listing",
        |mut host| {
            let folder = fixture();
            let long: String = (0..400).map(|n| format!("line {n}\n")).collect();
            std::fs::write(folder.path().join("long.txt"), long).expect("write long.txt");
            host.app().open(Location::local(folder.path()));
            let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
            assert!(host.run_until(PATIENCE, counted));
            while host.turn() {}
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|settings| settings.tenxer_mode = true);
            });
            select(&mut host, &Location::local(folder.path().join("long.txt")));
            wait_for_code_preview(&mut host);
            focus_view(&mut host);
            let offset = |host: &Offscreen| {
                let code = host.app().preview.viewer.code_state().expect("source text");
                code.scroll_offset()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            for _ in 0..3 {
                press(&mut host, Key::Text("J".to_owned()), shift);
            }
            let down = offset(&host);
            assert!(down > 0.0, "J scrolls down");
            press(&mut host, Key::Text("K".to_owned()), shift);
            assert!(offset(&host) < down, "K scrolls up");
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );

            press(&mut host, Key::Text("l".to_owned()), Mods::default());
            assert_ne!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );
            for _ in 0..3 {
                press(&mut host, Key::Text("J".to_owned()), shift);
            }
            assert!(offset(&host) > 0.0, "J scrolls inside the preview too");
            press(&mut host, Key::Text("g".to_owned()), Mods::default());
            press(&mut host, Key::Text("g".to_owned()), Mods::default());
            let top = host.run_until(PATIENCE, |host| offset(host) == 0.0);
            assert!(top, "g g goes to the top");
        },
    );
}
