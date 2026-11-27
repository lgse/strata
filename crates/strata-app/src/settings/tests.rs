// SPDX-License-Identifier: MIT

use strata_components::scroll::BarLook;
use strata_desktop::settings::{
    CornerRoundness, Preferences, ScrollbarHoverWidth, default_corner_roundness, settings_path,
};
use strata_kit::{shape, slots::scroll as scroll_tokens};

use crate::{
    host::offscreen::Offscreen,
    shell,
    startup::{self, UiThread},
    test_support::{PATIENCE, in_own_process},
};

const MODULE: &str = "settings::tests";

fn change(host: &mut Offscreen, change: impl FnOnce(&mut Preferences) + 'static) {
    host.runtime()
        .update(move |app, _| shell::saved(app).change(change));
}

fn reads(host: &mut Offscreen, wanted: impl Fn(&Preferences) -> bool) -> bool {
    host.run_until(PATIENCE, |host| wanted(shell::preferences(host.app())))
}

#[test]
fn the_windows_of_a_process_share_their_settings_and_their_saves() {
    in_own_process(
        &format!("{MODULE}::the_windows_of_a_process_share_their_settings_and_their_saves"),
        &[],
        || {
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, settings);
            let mut second = Offscreen::new(ui, shell::share_settings(first.app()));
            let peeking = !shell::preferences(first.app()).folder_peeking;
            let searching = !shell::preferences(first.app()).type_to_search;

            change(&mut first, move |saved| saved.folder_peeking = peeking);
            assert!(
                reads(&mut second, |saved| saved.folder_peeking == peeking),
                "the second window never read the first window's change"
            );
            change(&mut second, move |saved| saved.type_to_search = searching);
            assert!(
                reads(&mut first, |saved| saved.type_to_search == searching
                    && saved.folder_peeking == peeking),
                "the first window never read the second window's change"
            );

            strata_core::storage::writer::flush_at_exit();
            let text = std::fs::read_to_string(settings_path()).expect("the saved settings");
            let saved: Preferences = toml::from_str(&text).expect("settings the file holds");
            assert_eq!(
                (saved.folder_peeking, saved.type_to_search),
                (peeking, searching)
            );
        },
    );
}

/// The store holds no way into a window that has closed, and tells it nothing.
#[test]
fn a_closed_window_is_told_nothing() {
    in_own_process(
        &format!("{MODULE}::a_closed_window_is_told_nothing"),
        &[],
        || {
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, settings);
            let second = Offscreen::new(ui, shell::share_settings(first.app()));
            let mut told = 0;
            first
                .runtime()
                .update(|app, _| told = shell::saved(app).windows_told());
            assert_eq!(told, 2);
            drop(second);
            let peeking = !shell::preferences(first.app()).folder_peeking;

            change(&mut first, move |saved| saved.folder_peeking = peeking);

            assert!(reads(&mut first, |saved| saved.folder_peeking == peeking));
            first
                .runtime()
                .update(|app, _| told = shell::saved(app).windows_told());
            assert_eq!(told, 1);
        },
    );
}

#[test]
fn scrollbar_hover_width_applies_at_startup_and_propagates_across_windows() {
    in_own_process(
        &format!(
            "{MODULE}::scrollbar_hover_width_applies_at_startup_and_propagates_across_windows"
        ),
        &[],
        || {
            let path = settings_path();
            std::fs::create_dir_all(path.parent().expect("config dir")).expect("create config dir");
            std::fs::write(
                &path,
                "mode = \"theme\"\ntheme = \"tokyo-night\"\nscrollbar_expand_on_hover = true\nscrollbar_hover_width = 18\n",
            )
            .expect("write startup settings");

            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, settings);
            let mut second = Offscreen::new(ui, shell::share_settings(first.app()));

            for host in [&first, &second] {
                assert_eq!(scroll_tokens::hover_width(&host.app().look), Some(18.0));
                assert_eq!(BarLook::app(&host.app().look).hover_thickness, Some(18.0));
                assert_eq!(
                    BarLook::listing(&host.app().look).hover_thickness,
                    Some(18.0)
                );
            }

            first.runtime().update(|app, tx| {
                shell::saved(app)
                    .change(|saved| saved.scrollbar_hover_width = ScrollbarHoverWidth::new(22));
                shell::follow_look_tx(app, tx);
            });
            assert!(
                second.run_until(PATIENCE, |host| {
                    scroll_tokens::hover_width(&host.app().look) == Some(22.0)
                }),
                "second window adopts live scrollbar hover width change"
            );

            second.runtime().update(|app, tx| {
                shell::saved(app).change(|saved| saved.scrollbar_expand_on_hover = false);
                shell::follow_look_tx(app, tx);
            });
            assert!(
                first.run_until(PATIENCE, |host| {
                    scroll_tokens::hover_width(&host.app().look).is_none()
                }),
                "first window clears scrollbar hover width when disabled in another window"
            );
        },
    );
}

#[test]
fn scrollbar_hover_defaults_apply_at_startup_before_settings_opens() {
    in_own_process(
        &format!("{MODULE}::scrollbar_hover_defaults_apply_at_startup_before_settings_opens"),
        &[],
        || {
            let path = settings_path();
            std::fs::create_dir_all(path.parent().expect("config dir")).expect("create config dir");
            std::fs::write(&path, "mode = \"theme\"\ntheme = \"tokyo-night\"\n")
                .expect("write startup settings omitting scrollbar keys");

            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut host = Offscreen::new(ui, settings);

            assert!(shell::preferences(host.app()).scrollbar_expand_on_hover);
            assert_eq!(
                shell::preferences(host.app()).scrollbar_hover_width.get(),
                8
            );
            assert_eq!(scroll_tokens::hover_width(&host.app().look), Some(8.0));
            assert_eq!(BarLook::app(&host.app().look).hover_thickness, Some(8.0));
            assert_eq!(
                BarLook::listing(&host.app().look).hover_thickness,
                Some(8.0)
            );

            host.runtime().update(|app, tx| {
                shell::saved(app).change(|saved| saved.scrollbar_expand_on_hover = false);
                shell::follow_look_tx(app, tx);
            });
            assert!(scroll_tokens::hover_width(&host.app().look).is_none());

            strata_core::storage::writer::flush_at_exit();
            let text = std::fs::read_to_string(&path).expect("read persisted settings");
            let saved: Preferences = toml::from_str(&text).expect("parse persisted settings");
            assert!(!saved.scrollbar_expand_on_hover);
        },
    );
}

#[test]
fn scrollbar_hover_explicit_false_clears_look_at_startup() {
    in_own_process(
        &format!("{MODULE}::scrollbar_hover_explicit_false_clears_look_at_startup"),
        &[],
        || {
            let path = settings_path();
            std::fs::create_dir_all(path.parent().expect("config dir")).expect("create config dir");
            std::fs::write(
                &path,
                "mode = \"theme\"\ntheme = \"tokyo-night\"\nscrollbar_expand_on_hover = false\n",
            )
            .expect("write startup settings disabling scrollbar hover expansion");

            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let host = Offscreen::new(ui, settings);

            assert!(!shell::preferences(host.app()).scrollbar_expand_on_hover);
            assert_eq!(scroll_tokens::hover_width(&host.app().look), None);
            assert_eq!(BarLook::app(&host.app().look).hover_thickness, None);
            assert_eq!(BarLook::listing(&host.app().look).hover_thickness, None);
        },
    );
}

#[test]
fn corner_roundness_defaults_propagate_and_persist_changes() {
    in_own_process(
        &format!("{MODULE}::corner_roundness_defaults_propagate_and_persist_changes"),
        &[],
        || {
            let path = settings_path();
            std::fs::create_dir_all(path.parent().expect("config dir")).expect("create config dir");
            std::fs::write(&path, "mode = \"theme\"\ntheme = \"tokyo-night\"\n")
                .expect("write startup settings");

            let expected_default = default_corner_roundness();
            let expected_factor = shape::factor(expected_default.percent() as u16);
            let target = if expected_default == CornerRoundness::SQUARE {
                CornerRoundness::DEFAULT
            } else {
                CornerRoundness::new(100)
            };
            let target_factor = shape::factor(target.percent() as u16);

            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, settings);
            let mut second = Offscreen::new(ui, shell::share_settings(first.app()));

            for host in [&first, &second] {
                assert_eq!(
                    shell::preferences(host.app()).corner_roundness,
                    expected_default
                );
                assert_eq!(host.app().look.roundness, expected_factor);
            }

            first.runtime().update(move |app, tx| {
                shell::saved(app).change(move |saved| saved.corner_roundness = target);
                shell::follow_look_tx(app, tx);
            });
            assert!(
                second.run_until(PATIENCE, |host| {
                    (host.app().look.roundness - target_factor).abs() < f32::EPSILON
                }),
                "second window adopts live corner roundness change"
            );

            strata_core::storage::writer::flush_at_exit();
            let table: toml::Table = std::fs::read_to_string(&path)
                .expect("read saved settings")
                .parse()
                .expect("parse saved settings");
            assert_eq!(
                table
                    .get("corner_roundness")
                    .and_then(toml::Value::as_integer),
                Some(i64::from(target.percent())),
                "changing corner_roundness from default persists to settings.toml"
            );
        },
    );
}

#[test]
fn explicit_corner_roundness_overrides_default_at_startup() {
    in_own_process(
        &format!("{MODULE}::explicit_corner_roundness_overrides_default_at_startup"),
        &[],
        || {
            let path = settings_path();
            std::fs::create_dir_all(path.parent().expect("config dir")).expect("create config dir");
            std::fs::write(
                &path,
                "mode = \"theme\"\ntheme = \"tokyo-night\"\ncorner_roundness = 60\n",
            )
            .expect("write startup settings");

            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            let host = Offscreen::new(ui, settings);

            assert_eq!(
                shell::preferences(host.app()).corner_roundness,
                CornerRoundness::new(60)
            );
            assert_eq!(host.app().look.roundness, shape::factor(60));
        },
    );
}
