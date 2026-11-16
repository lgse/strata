// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;
use strata_core::util::{DateDisplay, modified_date_for_seconds};
use strata_preview::sandbox::MediaPreviewBackend;

use super::SettingsDialog;
use crate::{
    commands::Holds,
    host::offscreen::Offscreen,
    shell,
    slots::Part,
    test_support::{
        PATIENCE, hosted, send_key,
        view::{self, Shape, hosted_with, idle, open},
    },
};

const LIST: Shape = Shape {
    described: "List view",
    list: "list",
    entry: "list-item",
};

const MODULE: &str = "settings_dialog::tests";

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    idle(host);
}

fn click_at(host: &mut Offscreen, x: f32, y: f32) {
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn node_bounds(tree: &Value, key: &str) -> Option<[f32; 4]> {
    let nodes = view::all(tree, &|n| n["key"] == key);
    let b = &nodes.first()?["bounds"];
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    Some([
        b[0].as_f64()? as f32,
        b[1].as_f64()? as f32,
        b[2].as_f64()? as f32,
        b[3].as_f64()? as f32,
    ])
}

fn in_viewport([_, y, w, h]: [f32; 4], pane: Option<[f32; 4]>) -> bool {
    let sized = w > 0.0 && h > 0.0;
    sized && pane.is_none_or(|[_, py, _, ph]| y >= py && y + h <= py + ph)
}

fn scroll_step(y: f32, pane: Option<[f32; 4]>) -> (f32, f32, f32) {
    match pane {
        Some([px, py, pw, ph]) => {
            let dir = if y < py { 160.0 } else { -160.0 };
            (px + pw - 8.0, py + ph / 2.0, dir)
        }
        None => (600.0, 380.0, -160.0),
    }
}

fn reveal_and_center(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let mounted = |host: &Offscreen| {
        host.elements()
            .iter()
            .any(|el| !el.exiting && el.key.as_deref() == Some(key))
    };
    assert!(host.run_until(PATIENCE, mounted), "{key} is mounted");
    for _ in 0..24 {
        let tree = host.tree();
        let rect = node_bounds(&tree, key).expect("node bounds");
        let pane = node_bounds(&tree, "settings-page");
        if in_viewport(rect, pane) {
            return (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        }
        let (wx, wy, dy) = scroll_step(rect[1], pane);
        host.runtime().wheel(wx, wy, dy);
        idle(host);
    }
    panic!("{key} never scrolled into settings-page viewport");
}

fn click(host: &mut Offscreen, key: &str) {
    let (x, y) = reveal_and_center(host, key);
    click_at(host, x, y);
}

fn has_state(node: &Value, state: &str) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|states| states.iter().any(|held| held == state))
}

#[test]
fn ctrl_comma_opens_settings_focuses_general_and_dismisses_on_escape_or_outside_click() {
    hosted(
        MODULE,
        "ctrl_comma_opens_settings_focuses_general_and_dismisses_on_escape_or_outside_click",
        |mut host| {
            assert!(!host.app().settings_dialog.is_open());
            assert_eq!(SettingsDialog::keyboard(host.app()), None);

            press(
                &mut host,
                Key::Text(",".into()),
                Mods {
                    ctrl: true,
                    ..Mods::default()
                },
            );
            assert!(host.app().settings_dialog.is_open());
            assert_eq!(
                SettingsDialog::keyboard(host.app()),
                Some(Holds::Modal { field: false })
            );
            let tree = host.tree();
            let focused = view::all(&tree, &|node| has_state(node, "focused"));
            assert_eq!(focused.len(), 1);
            assert_eq!(focused[0]["key"], "settings-nav-general");

            // A click inside the panel keeps Settings open.
            click_at(&mut host, 600.0, 300.0);
            assert!(host.app().settings_dialog.is_open());

            // A click outside the panel dismisses Settings.
            click_at(&mut host, 5.0, 5.0);
            assert!(!host.app().settings_dialog.is_open());

            // Header button also opens Settings, and Escape closes it.
            click(&mut host, "header-settings");
            assert!(host.app().settings_dialog.is_open());
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
        },
    );
}

#[test]
fn choice_buttons_describe_value_and_picking_date_format_updates_live_format() {
    hosted(
        MODULE,
        "choice_buttons_describe_value_and_picking_date_format_updates_live_format",
        |mut host| {
            click(&mut host, "header-settings");
            let tree = host.tree();
            let date_btns = view::all(&tree, &|node| {
                node["key"] == "settings-choice-date-format"
                    || node["key"] == "settings-choice-date-format-toggle"
            });
            assert_eq!(date_btns.len(), 2);
            for node in &date_btns {
                assert_eq!(view::text(node, "name"), "Modified date format");
                assert_eq!(view::text(node, "description"), "Relative");
            }

            click(&mut host, "settings-choice-date-format-toggle");
            let tree = host.tree();
            let rel = view::all(&tree, &|node| {
                node["role"] == "menu-item-radio" && view::text(node, "name") == "Relative"
            });
            assert_eq!(rel.len(), 1);
            assert!(has_state(rel[0], "checked"));
            let iso = view::all(&tree, &|node| {
                node["role"] == "menu-item-radio" && view::text(node, "name") == "ISO 8601"
            });
            assert_eq!(iso.len(), 1);
            assert!(!has_state(iso[0], "checked"));

            click(&mut host, "settings-choice-date-format-opt-1");
            assert!(
                modified_date_for_seconds(0, DateDisplay::Preferred).contains('-'),
                "ISO 8601 date format applied live"
            );
            assert_eq!(shell::preferences(host.app()).date_format, "iso");

            let tree = host.tree();
            let updated = view::all(&tree, &|node| {
                node["key"] == "settings-choice-date-format-toggle"
            });
            assert_eq!(view::text(updated[0], "description"), "ISO 8601");
        },
    );
}

#[test]
fn appearance_switches_update_thumbnails_stl_worn_and_theme_look() {
    hosted(
        MODULE,
        "appearance_switches_update_thumbnails_stl_worn_and_theme_look",
        |mut host| {
            click(&mut host, "header-settings");
            click(&mut host, "settings-nav-theme");

            let before_bg = host.app().look.kit.c.bg_base;
            let (tx, ty) = reveal_and_center(&mut host, "settings-theme-catalog-scroll");
            for _ in 0..24 {
                let tree = host.tree();
                let card = node_bounds(&tree, "settings-theme-card-catppuccin-latte");
                let scroll = node_bounds(&tree, "settings-theme-catalog-scroll");
                if let Some([cx, cy, cw, ch]) = card
                    && in_viewport([cx, cy, cw, ch], scroll)
                {
                    click_at(&mut host, cx + cw / 2.0, cy + ch / 2.0);
                    break;
                }
                host.runtime().wheel(tx, ty, -120.0);
                idle(&mut host);
            }
            assert_eq!(shell::preferences(host.app()).theme, "catppuccin-latte");
            assert_ne!(host.app().look.kit.c.bg_base, before_bg);

            assert!(!shell::worn(host.app()).stl_thumbnails());
            click(&mut host, "settings-switch-stl-thumbnails");
            assert!(shell::preferences(host.app()).stl_thumbnails);
            assert!(shell::worn(host.app()).stl_thumbnails());

            assert!(shell::thumbnails(host.app()).enabled());
            click(&mut host, "settings-switch-show-thumbnails");
            assert!(!shell::thumbnails(host.app()).enabled());
        },
    );
}

fn cell_text(tree: &Value, suffix: &str) -> Option<String> {
    let nodes = view::all(tree, &|node| {
        node["key"]
            .as_str()
            .is_some_and(|key| key.ends_with(suffix))
    });
    let value = view::text(nodes.first()?, "name").to_owned();
    (!value.is_empty() && value != "—").then_some(value)
}

fn pick_date_format(host: &mut Offscreen, opt_index: usize) {
    click(host, "header-settings");
    click(host, "settings-choice-date-format-toggle");
    click(
        host,
        &format!("settings-choice-date-format-opt-{opt_index}"),
    );
    press(host, Key::Escape, Mods::default());
}

#[test]
fn date_format_updates_list_modified_cells_relative_timer_and_open_preview_header() {
    hosted_with(
        MODULE,
        "list",
        "date_format = \"iso\"\n".to_owned(),
        "date_format_updates_list_modified_cells_relative_timer_and_open_preview_header",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            std::fs::write(folder.path().join("note.txt"), "hello\n").expect("write");
            open(&mut host, LIST, folder.path());
            assert!(!host.app().list.is_relative_dates());
            assert!(host.run_until(PATIENCE, |h| {
                cell_text(&h.tree(), "cell-modified").is_some()
            }));

            press(&mut host, Key::Text(" ".into()), Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                cell_text(&h.tree(), "preview-detail-1").is_some()
            }));
            let tree = host.tree();
            let iso_cell = cell_text(&tree, "cell-modified").expect("iso cell");
            let iso_preview = cell_text(&tree, "preview-detail-1").expect("iso preview");
            assert!(iso_cell.contains('-'), "expected ISO cell, got {iso_cell}");
            assert_eq!(iso_preview, iso_cell);

            // Switch to Long: List cell and open Preview header update immediately.
            pick_date_format(&mut host, 2);
            assert!(!host.app().list.is_relative_dates());
            let tree = host.tree();
            let long_cell = cell_text(&tree, "cell-modified").expect("long cell");
            let long_preview = cell_text(&tree, "preview-detail-1").expect("long preview");
            assert_ne!(long_cell, iso_cell);
            assert_eq!(long_preview, long_cell);

            // Switch to Relative: enables relative timer and updates both surfaces.
            pick_date_format(&mut host, 0);
            assert!(host.app().list.is_relative_dates());
            let tree = host.tree();
            let rel_cell = cell_text(&tree, "cell-modified").expect("relative cell");
            let rel_preview = cell_text(&tree, "preview-detail-1").expect("relative preview");
            assert_ne!(rel_cell, long_cell);
            assert_eq!(rel_preview, rel_cell);
        },
    );
}

#[test]
fn video_preview_controls_sync_active_preview_media_backend() {
    hosted_with(
        MODULE,
        "columns",
        "hardware_accelerated_video_previews = true\nvideo_preview_backend = \"vaapi\"\n"
            .to_owned(),
        "video_preview_controls_sync_active_preview_media_backend",
        |mut host| {
            click(&mut host, "header-settings");
            let _ = reveal_and_center(&mut host, "settings-switch-video");
            let tree = host.tree();
            let accel_on = view::all(&tree, &|n| n["key"] == "settings-switch-video")
                .first()
                .is_some_and(|n| has_state(n, "checked"));
            if !accel_on {
                // Polaris GPU unavailable on this host: effective backend stays Software.
                assert_eq!(
                    host.app().preview.media_backend(),
                    MediaPreviewBackend::Software
                );
                return;
            }
            assert_eq!(
                host.app().preview.media_backend(),
                MediaPreviewBackend::VaApi
            );
            click(&mut host, "settings-choice-video-backend-toggle");
            click(&mut host, "settings-choice-video-backend-opt-2");
            assert_eq!(
                host.app().preview.media_backend(),
                MediaPreviewBackend::Vulkan
            );
            click(&mut host, "settings-switch-video");
            assert_eq!(
                host.app().preview.media_backend(),
                MediaPreviewBackend::Software
            );
        },
    );
}
