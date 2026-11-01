// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    test_support::gtk_test,
    ui::preferences::{CornerRoundness, fixtures::seed_omarchy_for_test},
};

#[test]
fn variant_choices_sync_across_windows_without_initialization_writes() {
    gtk_test(
        "ui::settings::theme::tests::variant_choices_sync_across_windows_without_initialization_writes",
        || {
            seed_omarchy_for_test();
            let preferences = PreferenceManager::shared();
            let themes = ThemeManager::shared();
            preferences.set_omarchy_variant(OmarchyVariant::Lighter);
            crate::storage::writer::settle();
            let settings = crate::ui::preferences::config_directory().join("settings.toml");
            let before = std::fs::read(&settings).expect("saved settings");
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let controls: Vec<_> = windows
                .iter()
                .map(|window| {
                    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    append_omarchy_variant_option(&content, &preferences, &themes);
                    window.set_child(Some(&content));
                    window.present();
                    let row = content.first_child().expect("variant row");
                    let button = row
                        .last_child()
                        .and_downcast::<gtk::MenuButton>()
                        .expect("variant choice");
                    (row, button)
                })
                .collect();
            assert_eq!(
                std::fs::read(&settings).expect("unchanged settings"),
                before
            );
            for (row, button) in &controls {
                assert!(row.is_visible() && row.is_sensitive());
                assert_eq!(button.label().as_deref(), Some("Lighter"));
            }
            let menu = controls[0]
                .1
                .popover()
                .expect("choice popover")
                .child()
                .expect("variant menu");
            menu.last_child()
                .and_downcast::<gtk::Button>()
                .expect("contrast option")
                .emit_clicked();
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Contrast);
            for (_, button) in &controls {
                assert_eq!(button.label().as_deref(), Some("Contrast"));
            }
            let switch_omarchy_theme = |name: &str| {
                let state = crate::ui::theme::omarchy_state_dir();
                std::fs::write(state.join("theme.name"), name).expect("switch Omarchy theme");
                themes.refresh_omarchy_state();
                crate::storage::writer::settle();
            };
            switch_omarchy_theme("other");
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Darker);
            for (_, button) in &controls {
                assert_eq!(button.label().as_deref(), Some("Darker"));
            }
            preferences.set_omarchy_variant(OmarchyVariant::Lighter);
            switch_omarchy_theme("fixture");
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Contrast);
            for (_, button) in &controls {
                assert_eq!(button.label().as_deref(), Some("Contrast"));
            }
            switch_omarchy_theme("other");
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Lighter);
            switch_omarchy_theme("fixture");
            themes.set_follow_omarchy(false);
            for (row, _) in &controls {
                assert!(!row.is_sensitive());
            }
            themes.set_follow_omarchy(true);
            menu.first_child()
                .and_downcast::<gtk::Button>()
                .expect("lighter option")
                .emit_clicked();
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Lighter);
            for (row, button) in &controls {
                assert!(row.is_sensitive());
                assert_eq!(button.label().as_deref(), Some("Lighter"));
            }
            for window in windows {
                window.close();
            }
        },
    );
}

#[test]
fn corner_roundness_controls_follow_the_preference_across_windows() {
    gtk_test(
        "ui::settings::theme::tests::corner_roundness_controls_follow_the_preference_across_windows",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_corner_roundness(CornerRoundness::new(40));
            crate::storage::writer::settle();
            let settings = crate::ui::preferences::config_directory().join("settings.toml");
            let before = std::fs::read(&settings).expect("saved settings");
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let controls: Vec<gtk::SpinButton> = windows
                .iter()
                .map(|window| {
                    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    append_corner_roundness_option(&content, &preferences);
                    window.set_child(Some(&content));
                    window.present();
                    content
                        .last_child()
                        .and_then(|group| group.first_child())
                        .and_then(|row| row.last_child())
                        .and_then(|controls| controls.first_child())
                        .and_downcast::<gtk::SpinButton>()
                        .expect("roundness control")
                })
                .collect();
            assert_eq!(
                std::fs::read(&settings).expect("unchanged settings"),
                before
            );
            for control in &controls {
                assert_eq!(control.value(), 40.0);
            }
            controls[0].set_value(130.0);
            assert_eq!(preferences.corner_roundness(), CornerRoundness::new(130));
            assert_eq!(controls[1].value(), 130.0);
            preferences.set_corner_roundness(CornerRoundness::new(0));
            for control in &controls {
                assert_eq!(control.value(), 0.0);
            }
            for window in windows {
                window.close();
            }
        },
    );
}

#[test]
fn variant_is_unavailable_without_omarchy() {
    gtk_test(
        "ui::settings::theme::tests::variant_is_unavailable_without_omarchy",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            append_omarchy_variant_option(
                &content,
                &PreferenceManager::shared(),
                &ThemeManager::shared(),
            );
            let row = content.first_child().expect("unavailable variant row");
            assert!(!row.is_visible());
            super::super::search::apply(&content, &super::super::search::State::default());
            assert!(!row.is_visible());
        },
    );
}

#[test]
fn interface_font_choices_follow_the_preference_across_windows() {
    gtk_test(
        "ui::settings::theme::tests::interface_font_choices_follow_the_preference_across_windows",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_interface_font(Some("Fixture Sans".into()));
            crate::storage::writer::settle();
            let settings = crate::ui::preferences::config_directory().join("settings.toml");
            let before = std::fs::read(&settings).expect("saved settings");
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let buttons: Vec<_> = windows
                .iter()
                .map(|window| {
                    let button = interface_font_choice(&preferences);
                    window.set_child(Some(&button));
                    window.present();
                    button
                })
                .collect();
            assert_eq!(
                std::fs::read(&settings).expect("saved settings"),
                before,
                "building the choice writes nothing"
            );
            for button in &buttons {
                assert_eq!(button.label().as_deref(), Some("Fixture Sans"));
            }

            let popover = buttons[0].popover().expect("font popover");
            popover.popup();
            let body = popover.child().and_downcast::<gtk::Box>().expect("body");
            let search = body
                .first_child()
                .and_downcast::<gtk::Entry>()
                .expect("search");
            let list = body
                .first_child()
                .and_then(|search| search.next_sibling())
                .and_downcast::<gtk::ScrolledWindow>()
                .and_then(|scroll| scroll.child())
                .and_downcast::<gtk::ListView>()
                .expect("font list");
            let model = list.model().expect("font model");
            let empty = body.last_child().expect("empty label");
            assert!(model.n_items() > 0, "the list includes Default");

            search.set_text("no font is named this");
            assert_eq!(model.n_items(), 0);
            assert!(empty.is_visible());
            search.set_text("");
            assert!(model.n_items() > 0);
            assert!(!empty.is_visible());

            list.emit_by_name::<()>("activate", &[&0u32]);
            assert_eq!(preferences.interface_font(), None);
            for button in &buttons {
                assert_eq!(button.label().as_deref(), Some("Default"));
            }

            preferences.set_interface_font(Some("Other Sans".into()));
            for button in &buttons {
                assert_eq!(button.label().as_deref(), Some("Other Sans"));
            }
            for window in windows {
                window.destroy();
            }
        },
    );
}
