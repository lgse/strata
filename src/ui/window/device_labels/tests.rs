// SPDX-License-Identifier: MIT

use super::*;

fn row_label(row: &gtk::Button) -> gtk::Label {
    row.child()
        .and_then(|content| content.last_child())
        .and_downcast::<gtk::Label>()
        .expect("displayed device label")
}

#[test]
fn saved_labels_apply_before_settings_live_across_windows_and_row_rebuilds() {
    crate::test_support::gtk_test(
        "ui::window::device_labels::tests::saved_labels_apply_before_settings_live_across_windows_and_row_rebuilds",
        || {
            crate::ui::preferences::PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            let id = "volume:fixture-kingston";
            let first = super::super::sidebar_button(
                crate::assets::icons::HARD_DRIVE,
                "Kingston system name",
            );
            let second = super::super::sidebar_button(
                crate::assets::icons::HARD_DRIVE,
                "Kingston system name",
            );
            bind_row_label(&first, &preferences, id, "Kingston system name");
            bind_row_label(&second, &preferences, id, "Kingston system name");
            let windows = [
                gtk::Window::builder().child(&first).build(),
                gtk::Window::builder().child(&second).build(),
            ];
            for window in &windows {
                window.present();
            }
            assert_eq!(row_label(&first).text(), "Research drive");
            assert_eq!(row_label(&second).text(), "Research drive");
            let other =
                super::super::sidebar_button(crate::assets::icons::HARD_DRIVE, "Other device");
            bind_row_label(
                &other,
                &preferences,
                "volume:fixture-sandisk",
                "Other device",
            );
            preferences.set_device_label(id, "Backup / photos 📁");
            assert_eq!(row_label(&first).text(), "Backup / photos 📁");
            assert_eq!(row_label(&second).text(), "Backup / photos 📁");
            assert_eq!(row_label(&other).text(), "Other device");
            let rebuilt = super::super::sidebar_button(
                crate::assets::icons::HARD_DRIVE,
                "Updated system name",
            );
            bind_row_label(&rebuilt, &preferences, id, "Updated system name");
            windows[0].set_child(Some(&rebuilt));
            assert_eq!(row_label(&rebuilt).text(), "Backup / photos 📁");
            preferences.set_device_label(id, "");
            assert_eq!(row_label(&rebuilt).text(), "Updated system name");
            assert_eq!(row_label(&second).text(), "Kingston system name");
            assert_eq!(row_label(&other).text(), "Other device");
            for window in windows {
                window.close();
            }
        },
    );
}
