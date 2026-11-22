// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use rune_fx::{ElectricStyle, FireStyle, Fx};
use serde_json::Value;
use strata_core::model::Location;
use strata_kit::{look::Look, shape::LookParams};

use super::*;
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, folder_of, hosted, send_key,
        view::{all, hosted_with, idle},
    },
};

const MODULE: &str = "tenxer_fx::tests";

fn toggle_tenxer(host: &mut Offscreen) {
    send_key(
        host,
        KeyEvent {
            key: Key::Command('m'),
            mods: Mods {
                ctrl: true,
                cmd: true,
                shift: true,
                ..Mods::default()
            },
        },
    );
    idle(host);
}

fn overlay_mounted(host: &Offscreen) -> bool {
    host.elements()
        .iter()
        .any(|el| !el.exiting && el.key.as_deref() == Some(OVERLAY_KEY))
}

fn open_and_focus(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = folder_of(&["alpha.txt", "beta.txt", "gamma.txt"]);
    host.app().open(Location::local(folder.path()));
    assert!(host.run_until(PATIENCE, |h| {
        h.app().browser().column_entry_counts(0).is_some()
    }));
    idle(host);
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    idle(host);
    folder
}

fn node_bounds(tree: &Value, key: &str) -> Option<[f32; 4]> {
    let nodes = all(tree, &|n| n["key"] == key);
    let b = &nodes.first()?["bounds"];
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    Some([
        b[0].as_f64()? as f32,
        b[1].as_f64()? as f32,
        b[2].as_f64()? as f32,
        b[3].as_f64()? as f32,
    ])
}

#[test]
fn entering_tenxer_mode_starts_plasma_splash_and_auto_unmounts_on_completion() {
    hosted(
        MODULE,
        "entering_tenxer_mode_starts_plasma_splash_and_auto_unmounts_on_completion",
        |mut host| {
            let _folder = open_and_focus(&mut host);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));

            toggle_tenxer(&mut host);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));

            // Advance virtual clock smoothly — 1.68 s plasma splash finishes & unmounts cleanly.
            let mut stepped = 0.0;
            while is_active(host.app()) && stepped < 2.2 {
                host.runtime().frame(0.032);
                stepped += 0.032;
            }
            assert!(
                !is_active(host.app()),
                "FX must finish within 2.2 s (stepped {stepped:.3}s)"
            );
            assert!(!overlay_mounted(&host), "FX overlay must unmount cleanly");
            assert!(
                host.app().tenxer_fx.splash_pixels.is_none(),
                "completed splash must release its Pixels handle"
            );

            // Turning 10xer mode off and back on, then off mid-flight cancels immediately.
            toggle_tenxer(&mut host);
            assert!(!shell::preferences(host.app()).tenxer_mode);
            toggle_tenxer(&mut host);
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));
            toggle_tenxer(&mut host);
            assert!(!shell::preferences(host.app()).tenxer_mode);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
            assert!(host.app().tenxer_fx.splash_pixels.is_none());
        },
    );
}

#[test]
fn starting_already_in_tenxer_mode_does_not_trigger_activation_fx() {
    hosted_with(
        MODULE,
        "columns",
        "tenxer_mode = true\n".to_owned(),
        "starting_already_in_tenxer_mode_does_not_trigger_activation_fx",
        |mut host| {
            idle(&mut host);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
        },
    );
}

#[test]
fn disabled_animations_mount_no_fx_overlay_and_cancel_active_splash() {
    hosted(
        MODULE,
        "disabled_animations_mount_no_fx_overlay_and_cancel_active_splash",
        |mut host| {
            let _folder = open_and_focus(&mut host);

            // Disabling animations mid-flight cancels an active splash cleanly.
            toggle_tenxer(&mut host);
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));
            host.runtime().update(|app, _| {
                shell::disable_animations(app);
            });
            idle(&mut host);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
            assert!(host.app().tenxer_fx.splash_pixels.is_none());

            // Exiting and re-entering 10xer mode while animations are disabled mounts no overlay.
            toggle_tenxer(&mut host);
            toggle_tenxer(&mut host);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));

            // Direct trigger is also suppressed while animations are disabled.
            host.runtime().update(|app, tx| {
                trigger(app, tx);
            });
            idle(&mut host);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
            assert!(host.app().tenxer_fx.splash_pixels.is_none());
        },
    );
}

#[test]
fn active_fx_overlay_never_blocks_keyboard_or_pointer_input_underneath() {
    hosted(
        MODULE,
        "active_fx_overlay_never_blocks_keyboard_or_pointer_input_underneath",
        |mut host| {
            let _folder = open_and_focus(&mut host);
            host.app().browser().select(0, 0);
            idle(&mut host);

            // Enter 10x mode and verify overlay is mounted and inert.
            toggle_tenxer(&mut host);
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));
            let overlay_info = host
                .elements()
                .into_iter()
                .find(|el| el.key.as_deref() == Some(OVERLAY_KEY))
                .expect("overlay element");
            assert!(overlay_info.inert, "FX overlay must be inert");

            // While the FX overlay is mid-flight, 10x keyboard navigation (`j`) works normally.
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text("j".into()),
                    mods: Mods::default(),
                },
            );
            idle(&mut host);
            assert_eq!(
                host.app().browser().focused_entry().map(|e| e.display_name),
                Some("beta.txt".to_owned()),
                "10x keyboard navigation must pass through active FX overlay"
            );

            // Clicking a row underneath the active FX overlay selects it normally.
            let tree = host.tree();
            let gamma = all(&tree, &|n| n["name"] == "gamma.txt");
            let bounds = &gamma.first().expect("gamma.txt row")["bounds"];
            #[expect(clippy::cast_possible_truncation, reason = "window points")]
            let (cx, cy) = (
                (bounds[0].as_f64().unwrap() + bounds[2].as_f64().unwrap() * 0.5) as f32,
                (bounds[1].as_f64().unwrap() + bounds[3].as_f64().unwrap() * 0.5) as f32,
            );
            assert!(
                is_active(host.app()),
                "FX still active before pointer click"
            );
            host.runtime().pointer_move(cx, cy);
            host.runtime().pointer_down(cx, cy);
            host.turn();
            host.runtime().pointer_up(cx, cy);
            idle(&mut host);
            assert_eq!(
                host.app().browser().focused_entry().map(|e| e.display_name),
                Some("gamma.txt".to_owned()),
                "pointer clicks must reach file rows underneath active FX overlay"
            );
        },
    );
}

#[test]
fn settings_tenxer_switch_triggers_plasma_splash_and_has_no_effect_picker() {
    hosted(
        MODULE,
        "settings_tenxer_switch_triggers_plasma_splash_and_has_no_effect_picker",
        |mut host| {
            // Open Settings via Ctrl+,
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Text(",".into()),
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                },
            );
            idle(&mut host);
            assert!(host.app().settings_dialog.is_open());

            // The temporary trial effect picker and preview button are no longer mounted.
            for removed_key in [
                "settings-choice-tenxer-fx-toggle",
                "settings-tenxer-fx-preview",
            ] {
                assert!(
                    !host
                        .elements()
                        .iter()
                        .any(|el| !el.exiting && el.key.as_deref() == Some(removed_key)),
                    "{removed_key} must not mount on General page"
                );
            }

            // Enabling 10xer mode in Settings triggers the plasma splash automatically.
            click_settings_control(&mut host, "settings-switch-tenxer");
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));

            // Disabling 10xer mode cancels the active splash immediately.
            click_settings_control(&mut host, "settings-switch-tenxer");
            assert!(!shell::preferences(host.app()).tenxer_mode);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
            assert!(host.app().tenxer_fx.splash_pixels.is_none());
        },
    );
}

fn active_fx_slots(host: &mut Offscreen) -> Vec<crate::fx::DecodedSlot> {
    let rt = host.runtime();
    crate::fx::collect_slots(&rt.instances, &rt.marks)
}

#[test]
fn plasma_splash_mounts_artwork_and_effect_slots_then_unmounts_to_zero() {
    hosted(
        MODULE,
        "plasma_splash_mounts_artwork_and_effect_slots_then_unmounts_to_zero",
        |mut host| {
            let _folder = open_and_focus(&mut host);

            // At rest: zero KIND_EFFECT slots and no splash GPU texture allocated.
            assert!(active_fx_slots(&mut host).is_empty());
            assert!(host.app().tenxer_fx.splash_pixels.is_none());

            // Trigger plasma splash and step into the entrance lightning slam (~0.24 s).
            host.runtime().update(|app, tx| {
                trigger(app, tx);
            });
            for _ in 0..12 {
                host.runtime().frame(0.02);
            }
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));
            assert!(
                host.elements()
                    .iter()
                    .any(|el| !el.exiting
                        && el.key.as_deref() == Some(plasma_splash::SPLASH_CARD_KEY)),
                "splash artwork card must mount while plasma splash is active"
            );
            let initial_sig = host
                .app()
                .tenxer_fx
                .splash_pixels
                .as_ref()
                .expect("splash Pixels cached during playback")
                .signature();

            let slam_slots = active_fx_slots(&mut host);
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Fire(p) if p.style == FireStyle::Border && p.swirl > 0.0
                )),
                "plasma splash must emit a swirling plasma border slot"
            );
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Electric(p) if p.style == ElectricStyle::Border
                )),
                "plasma splash must emit an overload electric border slot"
            );
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Electric(p) if p.style == ElectricStyle::Charged
                )),
                "plasma splash must emit a charged window perimeter slot"
            );
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Electric(p) if matches!(p.style, ElectricStyle::Arc { .. })
                )),
                "plasma splash must emit forking corner lightning arc slots"
            );
            assert!(
                slam_slots.iter().all(|s| s.layer.is_animating()),
                "every active plasma splash slot must animate"
            );

            // Stepping further reuses the exact same cached Pixels handle (zero re-uploads).
            host.runtime().frame(0.04);
            assert_eq!(
                host.app()
                    .tenxer_fx
                    .splash_pixels
                    .as_ref()
                    .map(Pixels::signature),
                Some(initial_sig),
                "splash Pixels must stay cached across animation frames"
            );

            // Step to completion: overlay unmounts, slots drop to 0, and Pixels is released.
            let mut stepped = 0.28;
            while is_active(host.app()) && stepped < 2.2 {
                host.runtime().frame(0.04);
                stepped += 0.04;
            }
            assert!(
                !is_active(host.app()),
                "plasma splash must finish within 2.2 s"
            );
            assert!(!overlay_mounted(&host));
            assert!(
                active_fx_slots(&mut host).is_empty(),
                "completed plasma splash must emit zero effect slots"
            );
            assert!(
                host.app().tenxer_fx.splash_pixels.is_none(),
                "completed plasma splash must release its Pixels handle"
            );

            // Disabled animations emits zero effect slots and allocates no Pixels.
            host.runtime().update(|app, tx| {
                shell::disable_animations(app);
                trigger(app, tx);
            });
            idle(&mut host);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
            assert!(active_fx_slots(&mut host).is_empty());
            assert!(host.app().tenxer_fx.splash_pixels.is_none());
        },
    );
}

#[test]
fn plasma_splash_renders_cleanly_in_dark_and_light_looks_and_roundness_extremes() {
    let splash = plasma_splash::create_pixels();
    assert!(splash.is_some(), "embedded splash artwork decodes cleanly");
    for dark in [true, false] {
        for roundness in [0_u16, 100] {
            let params = LookParams {
                roundness,
                ..LookParams::default()
            };
            let look = Look::fallback(dark).with_params(&params);
            for t in [0.0_f32, 0.25, 0.5, 0.8, 1.0] {
                let frame = FxFrame::new(&look, 1280.0, 800.0, t);
                let _ = plasma_splash::build(&frame, splash.as_ref());
            }
        }
    }
}

fn click_settings_control(host: &mut Offscreen, key: &str) {
    assert!(
        host.run_until(PATIENCE, |h| {
            h.elements()
                .iter()
                .any(|el| !el.exiting && el.key.as_deref() == Some(key))
        }),
        "{key} mounted"
    );
    for _ in 0..24 {
        let tree = host.tree();
        let rect = node_bounds(&tree, key).expect("control bounds");
        let pane = node_bounds(&tree, "settings-page");
        let visible = rect[2] > 0.0
            && rect[3] > 0.0
            && pane.is_none_or(|p| rect[1] >= p[1] && rect[1] + rect[3] <= p[1] + p[3]);
        if visible || key.contains("-opt-") {
            let cx = rect[0] + rect[2] * 0.5;
            let cy = rect[1] + rect[3] * 0.5;
            host.runtime().pointer_move(cx, cy);
            host.runtime().pointer_down(cx, cy);
            host.turn();
            host.runtime().pointer_up(cx, cy);
            idle(host);
            return;
        }
        let [px, py, pw, ph] = pane.expect("settings-page pane");
        let dy = if rect[1] < py { 160.0 } else { -160.0 };
        host.runtime().wheel(px + pw - 8.0, py + ph * 0.5, dy);
        idle(host);
    }
    panic!("{key} never became clickable");
}
