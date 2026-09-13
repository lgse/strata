// SPDX-License-Identifier: MIT

use super::super::*;
use crate::sandbox::MediaPreviewBackend;
use crate::test_support::gtk_test;
use crate::ui::theme::TextSize;

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
                vec![false, false, false, false, true, false]
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
            for (title, values) in [
                (
                    "Drag & drop to another device",
                    vec!["Always copy", "Always move", "Always ask"],
                ),
                (
                    "Auto-refresh folder",
                    vec!["1 min", "5 min", "10 min", "Off"],
                ),
            ] {
                for (index, value) in values.into_iter().enumerate() {
                    let page = if index % 2 == 0 { &first } else { &second };
                    let menu = descendants::<gtk::MenuButton>(page)
                        .into_iter()
                        .find(|menu| menu.tooltip_text().as_deref() == Some(title))
                        .expect("preference menu");
                    let option = descendants::<gtk::Button>(
                        menu.popover()
                            .expect("preference menu popover")
                            .upcast_ref(),
                    )
                    .into_iter()
                    .find(|button| {
                        descendants::<gtk::Label>(button.upcast_ref())
                            .iter()
                            .any(|label| label.text() == value)
                    })
                    .expect("menu choice");
                    option.emit_clicked();
                    for other in [&first, &second] {
                        assert!(
                            descendants::<gtk::MenuButton>(other)
                                .iter()
                                .any(|button| button.label().as_deref() == Some(value))
                        );
                    }
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
            let first = theme_page(manager.clone()).widget;
            let second = theme_page(manager.clone()).widget;
            let first_hints = keybindings_page(manager.clone());
            let second_hints = keybindings_page(manager.clone());
            let first_channel = channel_option(manager.clone(), None);
            let second_channel = channel_option(manager.clone(), None);
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
            assert_eq!(active_switches(&first), [true, false, true]);
            assert_eq!(active_switches(&second), [true, false, true]);
            manager.set_follow_omarchy(false);
            assert_eq!(active_switches(&first), [false, false, true]);
            assert_eq!(active_switches(&second), [false, false, true]);
            for (page, enabled) in [(&first, true), (&second, false)] {
                descendants::<gtk::Switch>(page)[1].set_active(enabled);
                assert_eq!(manager.element_glow(), enabled);
                assert_eq!(active_switches(&first), [false, enabled, true]);
                assert_eq!(active_switches(&first), active_switches(&second));
            }
            for pixels in [32, 11] {
                manager.set_text_size(TextSize::new(pixels));
                for page in [&first, &second] {
                    let control = descendants::<gtk::SpinButton>(page).remove(0);
                    assert_eq!(control.value_as_int(), pixels as i32);
                }
            }
            for page in [&first, &second] {
                let control = descendants::<gtk::SpinButton>(page).remove(0);
                control.set_value(27.0);
                assert_eq!(manager.text_size(), TextSize::new(27));
                for other in [&first, &second] {
                    assert_eq!(descendants::<gtk::SpinButton>(other)[0].value_as_int(), 27);
                }
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
            for page in [&first, &second] {
                descendants::<gtk::Entry>(page)
                    .into_iter()
                    .find(|entry| entry.has_css_class("theme-search"))
                    .expect("theme search entry")
                    .set_text("Synchronized fixture");
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
                    selected[0]
                        .parent()
                        .expect("theme card wrapper")
                        .is_child_visible()
                );
                let search = descendants::<gtk::Entry>(page)
                    .into_iter()
                    .find(|entry| entry.has_css_class("theme-search"))
                    .expect("theme search entry");
                search.set_text("no matching theme");
                assert!(
                    !selected[0]
                        .parent()
                        .expect("theme card wrapper")
                        .is_child_visible()
                );
                search.set_text("");
                assert!(
                    selected[0]
                        .parent()
                        .expect("theme card wrapper")
                        .is_child_visible()
                );
                assert!(
                    descendants::<gtk::Label>(selected[0].upcast_ref())
                        .iter()
                        .any(|label| label.text() == "Synchronized fixture")
                );
            }
            manager.set_follow_omarchy(true);
            let saved =
                std::fs::read_to_string(glib::user_config_dir().join("strata/settings.toml"))
                    .expect("saved preference fixture");
            std::fs::write(
                glib::home_dir().join(".local/state/omarchy/current/theme.name"),
                "live-swatch",
            )
            .expect("saved preference fixture");
            let loop_ = glib::MainLoop::new(None, false);
            let stop = loop_.clone();
            let pages = [first.clone(), second.clone()];
            let deadline = Instant::now() + Duration::from_secs(3);
            glib::timeout_add_local(Duration::from_millis(20), move || {
                let updated = pages.iter().all(|page| {
                    descendants::<gtk::Label>(page)
                        .iter()
                        .any(|label| label.text() == "Live Swatch")
                });
                if updated || Instant::now() >= deadline {
                    stop.quit();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
            loop_.run();
            for page in [&first, &second] {
                assert!(
                    descendants::<gtk::Label>(page)
                        .iter()
                        .any(|label| label.text() == "Live Swatch")
                );
            }
            assert_eq!(
                std::fs::read_to_string(glib::user_config_dir().join("strata/settings.toml"))
                    .expect("saved preference fixture"),
                saved
            );
            for page in [&first_hints, &second_hints] {
                let toggle = descendants::<gtk::Switch>(page).remove(0);
                toggle.set_active(!toggle.is_active());
                assert_eq!(
                    active_switches(&first_hints),
                    active_switches(&second_hints)
                );
            }
            for (page, value) in [
                (&first_channel, "Nightly"),
                (&second_channel, "Preview"),
                (&first_channel, "Stable"),
            ] {
                let menu = descendants::<gtk::MenuButton>(page.upcast_ref()).remove(0);
                let option = descendants::<gtk::Button>(
                    menu.popover()
                        .expect("preference menu popover")
                        .upcast_ref(),
                )
                .into_iter()
                .find(|button| {
                    descendants::<gtk::Label>(button.upcast_ref())
                        .iter()
                        .any(|label| label.text() == value)
                })
                .expect("channel choice");
                option.emit_clicked();
                for other in [&first_channel, &second_channel] {
                    assert_eq!(
                        descendants::<gtk::MenuButton>(other.upcast_ref())[0]
                            .label()
                            .as_deref(),
                        Some(value)
                    );
                }
            }
        },
    );
}
