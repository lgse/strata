// SPDX-License-Identifier: MIT

use std::fs;

use rune::{Key, KeyEvent, Mods, Tx};
use rune_fx::{ElectricStyle, FireStyle, Fx};
use serde_json::Value;
use strata_core::model::Location;
use strata_desktop::settings::{TenxerFx, parse_tenxer_fx, settings_path};
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
fn entering_tenxer_mode_starts_fx_and_auto_unmounts_within_one_second() {
    hosted(
        MODULE,
        "entering_tenxer_mode_starts_fx_and_auto_unmounts_within_one_second",
        |mut host| {
            let _folder = open_and_focus(&mut host);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));

            toggle_tenxer(&mut host);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));

            // Advance virtual clock smoothly in 16 ms frames — must complete & unmount within 1 s.
            let mut stepped = 0.0;
            while is_active(host.app()) && stepped < 1.0 {
                host.runtime().frame(0.016);
                stepped += 0.016;
            }
            assert!(
                !is_active(host.app()),
                "FX must finish within 1 s (stepped {stepped:.3}s)"
            );
            assert!(!overlay_mounted(&host), "FX overlay must unmount cleanly");
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
fn off_and_disabled_animations_mount_no_fx_overlay() {
    hosted(
        MODULE,
        "off_and_disabled_animations_mount_no_fx_overlay",
        |mut host| {
            let _folder = open_and_focus(&mut host);

            // Setting TenxerFx::Off suppresses FX on entering 10xer mode.
            host.runtime().update(|app, _| {
                shell::saved(app).set_tenxer_fx(TenxerFx::Off);
            });
            toggle_tenxer(&mut host);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));

            // Exit 10xer mode, re-enable a visual concept, then disable animations (reduced motion).
            toggle_tenxer(&mut host);
            host.runtime().update(|app, _| {
                shell::saved(app).set_tenxer_fx(TenxerFx::VelocityWarp);
                shell::disable_animations(app);
            });
            toggle_tenxer(&mut host);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));

            // Direct preview trigger is also suppressed while animations are disabled.
            host.runtime().update(|app, tx| {
                trigger(app, TenxerFx::TacticalHud, tx);
            });
            idle(&mut host);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
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
fn settings_fx_choice_triggers_live_preview_and_persists_selection() {
    hosted(
        MODULE,
        "settings_fx_choice_triggers_live_preview_and_persists_selection",
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

            let toggle_key = "settings-choice-tenxer-fx-toggle";
            assert!(
                host.elements()
                    .iter()
                    .any(|el| !el.exiting && el.key.as_deref() == Some(toggle_key)),
                "10x mode effect dropdown should mount on General page"
            );

            // Open the dropdown and select "Plasma splash" (option 6).
            click_settings_control(&mut host, toggle_key);
            click_settings_control(&mut host, "settings-choice-tenxer-fx-opt-6");

            assert_eq!(shell::tenxer_fx(host.app()), TenxerFx::PlasmaSplash);
            assert!(is_active(host.app()), "picking a concept previews it live");
            assert!(overlay_mounted(&host));

            // Wait for async settings write to hit disk and confirm persistence.
            assert!(
                host.run_until(PATIENCE, |_| {
                    fs::read_to_string(settings_path())
                        .ok()
                        .is_some_and(|s| parse_tenxer_fx(&s) == (TenxerFx::PlasmaSplash, false))
                }),
                "tenxer_fx persisted to settings.toml"
            );

            // Step preview to completion, then click the Preview effect replay button.
            let mut stepped = 0.0;
            while is_active(host.app()) && stepped < 2.2 {
                host.runtime().frame(0.05);
                stepped += 0.05;
            }
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
            click_settings_control(&mut host, "settings-tenxer-fx-preview");
            assert!(is_active(host.app()), "replay button triggers live preview");

            // Switching to Off cancels active preview immediately.
            let off_index = TenxerFx::ALL
                .iter()
                .position(|&fx| fx == TenxerFx::Off)
                .expect("Off in TenxerFx::ALL");
            click_settings_control(&mut host, toggle_key);
            click_settings_control(
                &mut host,
                &format!("settings-choice-tenxer-fx-opt-{off_index}"),
            );
            assert_eq!(shell::tenxer_fx(host.app()), TenxerFx::Off);
            assert!(!is_active(host.app()));
            assert!(!overlay_mounted(&host));
        },
    );
}

fn active_fx_slots(host: &mut Offscreen) -> Vec<crate::fx::DecodedSlot> {
    let rt = host.runtime();
    crate::fx::collect_slots(&rt.instances, &rt.marks)
}

#[test]
fn interface_on_fire_emits_effect_slots_during_playback_and_unmounts_to_zero() {
    hosted(
        MODULE,
        "interface_on_fire_emits_effect_slots_during_playback_and_unmounts_to_zero",
        |mut host| {
            let _folder = open_and_focus(&mut host);

            // At rest: zero KIND_EFFECT slots -> FxCompositor stays idle.
            assert!(
                active_fx_slots(&mut host).is_empty(),
                "idle frame must emit no effect slots"
            );

            // Trigger Interface on fire and step into the peak blaze window.
            host.runtime().update(|app, tx| {
                trigger(app, TenxerFx::InterfaceOnFire, tx);
            });
            for _ in 0..20 {
                host.runtime().frame(0.02);
            }
            assert!(is_active(host.app()));
            assert!(overlay_mounted(&host));

            let peak_slots = active_fx_slots(&mut host);
            assert_eq!(
                peak_slots.len(),
                5,
                "peak InterfaceOnFire frame must emit all 5 rune-fx slots (haze, border, bottom, badge flame, badge haze)"
            );
            assert!(
                peak_slots.iter().all(|s| s.layer.is_animating()),
                "all active fire slots must animate"
            );

            // Step to completion: overlay unmounts cleanly and returns to 0 effect slots.
            let mut stepped = 0.4;
            while is_active(host.app()) && stepped < 2.2 {
                host.runtime().frame(0.04);
                stepped += 0.04;
            }
            assert!(
                !is_active(host.app()),
                "InterfaceOnFire must finish within 2.2 s"
            );
            assert!(!overlay_mounted(&host));
            assert!(
                active_fx_slots(&mut host).is_empty(),
                "completed InterfaceOnFire must emit zero effect slots"
            );

            // Reduced motion / disabled animations emits zero effect slots.
            host.runtime().update(|app, tx| {
                shell::disable_animations(app);
                trigger(app, TenxerFx::InterfaceOnFire, tx);
            });
            idle(&mut host);
            assert!(
                active_fx_slots(&mut host).is_empty(),
                "disabled animations must emit no effect slots"
            );
        },
    );
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

            // Trigger PlasmaSplash and step into the entrance lightning slam (~0.24 s).
            host.runtime().update(|app, tx| {
                trigger(app, TenxerFx::PlasmaSplash, tx);
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
                "splash artwork card must mount while PlasmaSplash is active"
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
                "PlasmaSplash must emit a swirling plasma border slot"
            );
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Electric(p) if p.style == ElectricStyle::Border
                )),
                "PlasmaSplash must emit an overload electric border slot"
            );
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Electric(p) if p.style == ElectricStyle::Charged
                )),
                "PlasmaSplash must emit a charged window perimeter slot"
            );
            assert!(
                slam_slots.iter().any(|s| matches!(
                    s.layer.fx,
                    Fx::Electric(p) if matches!(p.style, ElectricStyle::Arc { .. })
                )),
                "PlasmaSplash must emit forking corner lightning arc slots"
            );
            assert!(
                slam_slots.iter().all(|s| s.layer.is_animating()),
                "every active PlasmaSplash slot must animate"
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
                "PlasmaSplash must finish within 2.2 s"
            );
            assert!(!overlay_mounted(&host));
            assert!(
                active_fx_slots(&mut host).is_empty(),
                "completed PlasmaSplash must emit zero effect slots"
            );
            assert!(
                host.app().tenxer_fx.splash_pixels.is_none(),
                "completed PlasmaSplash must release its Pixels handle"
            );

            // Disabled animations emits zero effect slots and allocates no Pixels.
            host.runtime().update(|app, tx| {
                shell::disable_animations(app);
                trigger(app, TenxerFx::PlasmaSplash, tx);
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
fn every_concept_renders_cleanly_in_dark_and_light_looks_and_roundness_extremes() {
    let splash = plasma_splash::create_pixels();
    assert!(splash.is_some(), "embedded splash artwork decodes cleanly");
    for dark in [true, false] {
        for roundness in [0_u16, 100] {
            let params = LookParams {
                roundness,
                ..LookParams::default()
            };
            let look = Look::fallback(dark).with_params(&params);
            for effect in TenxerFx::ALL {
                for t in [0.0_f32, 0.25, 0.5, 0.8, 1.0] {
                    let frame = FxFrame::new(&look, 1280.0, 800.0, t);
                    let built = render_effect(effect, &frame, splash.as_ref());
                    assert_eq!(built.is_some(), effect.is_enabled());
                }
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
