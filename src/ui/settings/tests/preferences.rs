// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::*;
use crate::test_support::gtk_test;

fn descendants<T: IsA<gtk::Widget> + Clone>(root: &gtk::Widget) -> Vec<T> {
    let mut widgets = Vec::new();
    if let Ok(widget) = root.clone().downcast::<T>() {
        widgets.push(widget);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        widgets.extend(descendants::<T>(&widget));
        child = widget.next_sibling();
    }
    widgets
}

fn active_switches(root: &gtk::Widget) -> Vec<bool> {
    descendants::<gtk::Switch>(root)
        .iter()
        .map(gtk::Switch::is_active)
        .collect()
}

fn active_choices(root: &gtk::Widget) -> Vec<bool> {
    descendants::<gtk::ToggleButton>(root)
        .iter()
        .map(gtk::ToggleButton::is_active)
        .collect()
}

#[test]
fn every_general_control_stays_in_sync_without_initializing_browser_behavior() {
    gtk_test(
        "ui::settings::tests::preferences::every_general_control_stays_in_sync_without_initializing_browser_behavior",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            let first_browser = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let second_browser = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let path = glib::user_config_dir().join("strata/settings.toml");
            let before = std::fs::read_to_string(&path).expect("saved settings");
            let (first, _, _) = general_page(manager.clone());
            let (second, _, _) = general_page(manager.clone());
            assert_eq!(
                std::fs::read_to_string(&path).expect("saved settings"),
                before
            );
            assert_eq!(
                active_switches(&first),
                vec![false, false, true, false, false, true]
            );
            assert_eq!(active_switches(&first), active_switches(&second));
            assert_eq!(active_choices(&first), active_choices(&second));
            for page in [&first, &second] {
                for toggle in descendants::<gtk::Switch>(page) {
                    toggle.set_active(!toggle.is_active());
                    assert_eq!(active_switches(&first), active_switches(&second));
                    first_browser.assert_saved_preferences(&manager);
                    second_browser.assert_saved_preferences(&manager);
                }
                for button in descendants::<gtk::ToggleButton>(page)
                    .into_iter()
                    .filter(|button| button.has_css_class("segmented-control-option"))
                {
                    button.set_active(true);
                    assert_eq!(active_choices(&first), active_choices(&second));
                    first_browser.assert_saved_preferences(&manager);
                    second_browser.assert_saved_preferences(&manager);
                }
            }
            manager.set_video_preview_backend(MediaPreviewBackend::VaApi);
            for page in [&first, &second] {
                let backend = descendants::<gtk::MenuButton>(page)
                    .into_iter()
                    .find(|button| button.label().as_deref() == Some("VA-API"))
                    .expect("backend label follows preferences");
                assert_eq!(
                    backend.is_sensitive(),
                    manager.hardware_accelerated_video_previews()
                );
            }
            let before = std::fs::read_to_string(&path).expect("saved settings");
            let (third, _, _) = general_page(manager.clone());
            assert_eq!(active_switches(&third), active_switches(&first));
            assert_eq!(active_choices(&third), active_choices(&first));
            assert_eq!(
                std::fs::read_to_string(path).expect("saved settings"),
                before
            );
        },
    );
}

#[test]
fn theme_hint_and_channel_controls_follow_external_changes() {
    gtk_test(
        "ui::settings::tests::preferences::theme_hint_and_channel_controls_follow_external_changes",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            ThemeManager::seed_omarchy_for_test();
            let manager = ThemeManager::shared();
            let (first, _) = theme_page(manager.clone());
            let (second, _) = theme_page(manager.clone());
            let first_hints = keybindings_page(manager.clone());
            let second_hints = keybindings_page(manager.clone());
            let (first_channel, _) = channel_option(manager.clone(), None);
            let (second_channel, _) = channel_option(manager.clone(), None);
            let updates = [
                automatic_updates_option(&manager, UpdateMethod::InPlace),
                automatic_updates_option(&manager, UpdateMethod::InPlace),
            ];
            assert_eq!(active_switches(updates[0].upcast_ref()), [false]);
            for row in &updates {
                let toggle = descendants::<gtk::Switch>(row.upcast_ref()).remove(0);
                toggle.set_active(!toggle.is_active());
                assert_eq!(
                    active_switches(updates[0].upcast_ref()),
                    active_switches(updates[1].upcast_ref())
                );
            }
            manager.set_follow_omarchy(true);
            assert_eq!(active_switches(&first), [true]);
            assert_eq!(active_switches(&second), [true]);
            manager.set_follow_omarchy(false);
            assert_eq!(active_switches(&first), [false]);
            assert_eq!(active_switches(&second), [false]);
            manager.set_text_size(TextSize::Small);
            for page in [&first, &second] {
                let small = descendants::<gtk::ToggleButton>(page)
                    .into_iter()
                    .find(|button| button.label().as_deref() == Some("Small"))
                    .expect("text size control");
                assert!(small.is_active());
                let selected_cards = descendants::<gtk::Button>(page)
                    .into_iter()
                    .filter(|button| {
                        button.has_css_class("theme-card") && button.has_css_class("selected")
                    })
                    .count();
                assert_eq!(selected_cards, 1);
            }
            manager.select_theme("azure-glow");
            for page in [&first, &second] {
                let selected = descendants::<gtk::Button>(page)
                    .into_iter()
                    .find(|button| {
                        button.has_css_class("theme-card") && button.has_css_class("selected")
                    })
                    .expect("selected theme");
                assert!(
                    descendants::<gtk::Label>(selected.upcast_ref())
                        .iter()
                        .any(|label| label.text() == "Azure Glow")
                );
            }
            let mut custom = manager.starter_tokens();
            custom.name = "Synchronized fixture".into();
            manager
                .save_custom_theme(custom)
                .expect("save shared custom theme");
            for page in [&first, &second] {
                let selected = descendants::<gtk::Button>(page)
                    .into_iter()
                    .filter(|button| {
                        button.has_css_class("theme-card") && button.has_css_class("selected")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(selected.len(), 1);
                assert!(
                    descendants::<gtk::Label>(selected[0].upcast_ref())
                        .iter()
                        .any(|label| label.text() == "Synchronized fixture")
                );
            }
            for page in [&first_hints, &second_hints] {
                let toggle = descendants::<gtk::Switch>(page).remove(0);
                toggle.set_active(!toggle.is_active());
                assert_eq!(
                    active_switches(&first_hints),
                    active_switches(&second_hints)
                );
            }
            for page in [&first_channel, &second_channel] {
                for button in descendants::<gtk::ToggleButton>(page.upcast_ref()) {
                    button.set_active(true);
                    assert_eq!(
                        active_choices(first_channel.upcast_ref()),
                        active_choices(second_channel.upcast_ref())
                    );
                }
            }
        },
    );
}
