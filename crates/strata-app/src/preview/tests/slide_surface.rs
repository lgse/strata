// SPDX-License-Identifier: MIT

use rune::{Color, Key, Mods, Tx};
use strata_core::model::Location;

use super::{disable_auto_preview, focus_view, listed, select, switch_view, wait_for_code_preview};
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{hosted, send_key},
};

const MODULE: &str = "preview::tests::slide_surface";

const SAMPLE_STEPS_S: [f64; 6] = [0.0, 1.0 / 60.0, 0.04, 0.08, 0.08, 0.08];

fn enable_transparency(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        shell::saved(app).change_transparency(|t| {
            t.enabled = true;
            t.background_transparency = 30;
            t.raised_surface_transparency = 40;
        });
        shell::follow_look_tx(app, tx);
    });
    while host.turn() {}
    assert!(
        host.app().look.kit.c.bg_base.a < 0.99 && host.app().look.kit.c.bg_deep.a < 0.99,
        "transparency must make bg_base and bg_deep translucent"
    );
}

fn approx_color(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 1e-3
        && (a.g - b.g).abs() < 1e-3
        && (a.b - b.b).abs() < 1e-3
        && (a.a - b.a).abs() < 1e-3
}

fn contains_half_open(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

fn assert_split_surface_coverage(host: &mut Offscreen, label: &str) {
    let bg_base = host.app().look.kit.c.bg_base;
    let bg_deep = host.app().look.kit.c.bg_deep;
    let elements = host.runtime().elements();

    let split = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some("preview-split"))
        .unwrap_or_else(|| panic!("{label}: preview-split missing"));
    let files = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some("preview-files"))
        .unwrap_or_else(|| panic!("{label}: preview-files missing"));
    let slot = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some("preview-slot"))
        .unwrap_or_else(|| panic!("{label}: preview-slot missing"));

    let [sx, sy, sw, sh] = split.visible;
    assert!(sw > 100.0 && sh > 100.0, "{label}: split rect {split:?}");

    let files_right = files.visible[0] + files.visible[2];
    if slot.visible[2] > 0.0 {
        let seam_gap = (files_right - slot.visible[0]).abs();
        assert!(
            seam_gap <= 0.5,
            "{label}: layout gap between preview-files (right={files_right}) and preview-slot \
             (left={})",
            slot.visible[0]
        );
    } else {
        let right_gap = (files_right - (sx + sw)).abs();
        assert!(
            right_gap <= 0.5,
            "{label}: preview-files right ({files_right}) does not reach split right ({})",
            sx + sw
        );
    }

    let is_foreground_accent = |el: &rune::ElementInfo| {
        el.key.as_deref().is_some_and(|key| {
            key == "badge"
                || key == "preview-header"
                || (key.starts_with("columns-col-") && key.ends_with("-header"))
        })
    };
    let split_prefix = format!("{}/", split.path);
    let surfaces: Vec<_> = elements
        .iter()
        .filter(|el| {
            (el.path == split.path || el.path.starts_with(&split_prefix))
                && !is_foreground_accent(el)
                && el.opacity > 0.0
                && el.visible[2] > 0.0
                && el.visible[3] > 0.0
                && (approx_color(el.fill, bg_base) || approx_color(el.fill, bg_deep))
        })
        .collect();

    let mut xs = Vec::new();
    let mut x = sx + 2.0;
    while x < sx + sw - 1.0 {
        xs.push(x);
        x += 25.0;
    }
    xs.push(sx + sw - 2.0);
    if files_right > sx + 2.0 && files_right < sx + sw - 2.0 {
        xs.push(files_right - 10.0);
        xs.push(files_right - 1.5);
    }
    if slot.visible[2] > 2.0 {
        xs.push(slot.visible[0] + 0.5);
        xs.push(slot.visible[0] + 2.0);
        xs.push(slot.visible[0] + slot.visible[2] * 0.5);
    }

    let ys = [
        sy + 12.0,
        sy + 52.0,
        sy + 100.0,
        sy + sh * 0.5,
        sy + sh - 16.0,
    ];

    for &py in &ys {
        if py <= sy || py >= sy + sh {
            continue;
        }
        for &px in &xs {
            if px <= sx || px >= sx + sw {
                continue;
            }
            let covering: Vec<_> = surfaces
                .iter()
                .filter(|el| contains_half_open(el.visible, px, py))
                .map(|el| (el.path.as_str(), el.visible, el.exiting, el.fill))
                .collect();
            assert_eq!(
                covering.len(),
                1,
                "{label}: at ({px:.1}, {py:.1}) inside split {:?} (files={:?}, slot={:?}) \
                 expected 1 surface fill, got {}: {covering:#?}\nall surfaces: {:#?}",
                split.visible,
                files.visible,
                slot.visible,
                covering.len(),
                surfaces
                    .iter()
                    .map(|el| (el.path.as_str(), el.visible, el.exiting))
                    .collect::<Vec<_>>(),
            );
        }
    }
}

fn inspect_slide(host: &mut Offscreen, phase: &str) {
    let mut saw_mid_slide = false;
    let mut elapsed = 0.0;
    for (idx, dt) in SAMPLE_STEPS_S.into_iter().enumerate() {
        host.runtime().frame(dt);
        elapsed += dt;
        let slot_w = host
            .runtime()
            .elements()
            .into_iter()
            .find(|el| !el.exiting && el.key.as_deref() == Some("preview-slot"))
            .map_or(0.0, |el| el.visible[2]);
        let slot_target_w = host
            .runtime()
            .elements()
            .into_iter()
            .find(|el| !el.exiting && el.key.as_deref() == Some("preview-slot"))
            .map_or(0.0, |el| el.target[2]);
        if (slot_w - slot_target_w).abs() > 5.0 {
            saw_mid_slide = true;
        }
        assert_split_surface_coverage(host, &format!("{phase} step={idx} t={elapsed:.3}s"));
    }
    assert!(
        saw_mid_slide,
        "{phase}: expected to observe intermediate animated slot widths"
    );
    while host.turn() {}
    assert_split_surface_coverage(host, &format!("{phase} settled"));
}

#[test]
fn list_preview_open_and_close_slides_cover_surface_without_gaps_or_double_blending() {
    hosted(
        MODULE,
        "list_preview_open_and_close_slides_cover_surface_without_gaps_or_double_blending",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            enable_transparency(&mut host);
            switch_view(&mut host, "list");
            let todo = Location::local(folder.path().join("todo.txt"));
            select(&mut host, &todo);
            focus_view(&mut host);
            assert_split_surface_coverage(&mut host, "list closed initial");

            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            inspect_slide(&mut host, "list open");
            wait_for_code_preview(&mut host);

            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            inspect_slide(&mut host, "list close");

            // Mid-slide reversal: open, step halfway, then close before the open settles.
            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            host.runtime().frame(0.0);
            assert_split_surface_coverage(&mut host, "list reverse open t=0");
            host.runtime().frame(0.10);
            assert_split_surface_coverage(&mut host, "list reverse open mid");
            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            inspect_slide(&mut host, "list reverse close");
        },
    );
}

#[test]
fn icons_preview_open_and_close_slides_cover_surface_without_gaps_or_double_blending() {
    hosted(
        MODULE,
        "icons_preview_open_and_close_slides_cover_surface_without_gaps_or_double_blending",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            enable_transparency(&mut host);
            switch_view(&mut host, "icons");
            let todo = Location::local(folder.path().join("todo.txt"));
            select(&mut host, &todo);
            focus_view(&mut host);
            assert_split_surface_coverage(&mut host, "icons closed initial");

            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            inspect_slide(&mut host, "icons open");
            wait_for_code_preview(&mut host);

            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            inspect_slide(&mut host, "icons close");
        },
    );
}

#[test]
fn columns_unreserved_preview_close_and_empty_slot_release_cover_surface_without_gaps() {
    hosted(
        MODULE,
        "columns_unreserved_preview_close_and_empty_slot_release_cover_surface_without_gaps",
        |mut host| {
            let folder = listed(&mut host);
            disable_auto_preview(&mut host);
            enable_transparency(&mut host);
            let todo = Location::local(folder.path().join("todo.txt"));
            select(&mut host, &todo);
            focus_view(&mut host);

            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Text(" ".to_owned()),
                    mods: Mods::default(),
                },
            );
            wait_for_code_preview(&mut host);
            assert_split_surface_coverage(&mut host, "columns open");

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                let (depth, entry) = crate::preview::session::selected_entry(app);
                crate::preview::session::toggle_panel(app, entry, depth, tx);
            });
            inspect_slide(&mut host, "columns toggle panel off");

            // Re-enable panel, dismiss to empty reserved slot, then release the empty reserved slot.
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                let (depth, entry) = crate::preview::session::selected_entry(app);
                crate::preview::session::toggle_panel(app, entry, depth, tx);
            });
            while host.turn() {}
            send_key(
                &mut host,
                rune::KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            while host.turn() {}
            assert_split_surface_coverage(&mut host, "columns empty reserved slot");

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                let (depth, entry) = crate::preview::session::selected_entry(app);
                crate::preview::session::toggle_panel(app, entry, depth, tx);
            });
            inspect_slide(&mut host, "columns empty reserved slot release");
        },
    );
}
