// SPDX-License-Identifier: MIT

use super::*;
use crate::{test_support::gtk_test, ui::preferences::fixtures::seed_omarchy_for_test};

#[test]
fn variant_choices_sync_across_windows_without_initialization_writes() {
    gtk_test(
        "ui::settings::theme::tests::variant_choices_sync_across_windows_without_initialization_writes",
        || {
            seed_omarchy_for_test();
            let preferences = PreferenceManager::shared();
            let themes = ThemeManager::shared();
            preferences.set_omarchy_variant(OmarchyVariant::Darker);
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
                assert_eq!(button.label().as_deref(), Some("Darker"));
            }
            let menu = controls[0]
                .1
                .popover()
                .expect("choice popover")
                .child()
                .expect("variant menu");
            menu.last_child()
                .and_downcast::<gtk::Button>()
                .expect("high contrast option")
                .emit_clicked();
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::HighContrast);
            for (_, button) in &controls {
                assert_eq!(button.label().as_deref(), Some("High contrast"));
            }
            themes.set_follow_omarchy(false);
            for (row, _) in &controls {
                assert!(!row.is_sensitive());
            }
            themes.set_follow_omarchy(true);
            menu.first_child()
                .and_downcast::<gtk::Button>()
                .expect("original option")
                .emit_clicked();
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Original);
            for (row, button) in &controls {
                assert!(row.is_sensitive());
                assert_eq!(button.label().as_deref(), Some("Original"));
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
