// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser_modes::{BrowserDensity, BrowserMode, ClickActivation, ClickCount};

#[test]
fn live_preferences_reach_existing_and_future_browsers_but_preserve_chooser_policy() {
    gtk_test(
        "ui::window::tests::preferences::live_preferences_reach_existing_and_future_browsers_but_preserve_chooser_policy",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            let first = browser_for_window();
            let second = browser_for_window();
            let chooser = crate::ui::browser::BrowserView::new_chooser(
                std::rc::Rc::new(crate::adapters::LocalFileSource),
                false,
            );
            for enabled in [true, false, true] {
                manager.set_folder_peeking(enabled);
                manager.set_single_click_previews(enabled);
                manager.set_group_by_type(enabled);
                manager.set_auto_refresh_interval(if enabled { 60 } else { 0 });
                manager.set_browser_density(if enabled {
                    BrowserDensity::Compact
                } else {
                    BrowserDensity::Airy
                });
                for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                    manager.set_browser_mode(mode);
                    manager.set_click_activation(
                        mode,
                        ClickActivation {
                            files: if enabled {
                                ClickCount::Two
                            } else {
                                ClickCount::One
                            },
                            folders: if enabled {
                                ClickCount::One
                            } else {
                                ClickCount::Two
                            },
                        },
                    );
                    for view in [&first, &second] {
                        assert_eq!(view.view_mode(), mode);
                        view.assert_saved_preferences(&manager);
                        view.assert_peek_scheduling(enabled);
                    }
                    assert_eq!(chooser.view_mode(), mode);
                    chooser.assert_peek_scheduling(false);
                }
                let third = browser_for_window();
                third.assert_saved_preferences(&manager);
                assert_eq!(third.view_mode(), manager.browser_mode());
            }
            first.browser().toggle_hidden();
            assert_eq!(
                second.browser().preferences(),
                first.browser().preferences()
            );
            assert_eq!(
                chooser.browser().preferences(),
                first.browser().preferences()
            );
        },
    );
}

#[test]
fn sidebar_order_and_update_notices_follow_preferences_without_settings() {
    use std::rc::Rc;
    gtk_test(
        "ui::window::tests::preferences::sidebar_order_and_update_notices_follow_preferences_without_settings",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            let first = super::super::build_sidebar(browser_for_window(), manager.clone(), true);
            let second = super::super::build_sidebar(browser_for_window(), manager.clone(), true);
            manager.set_sidebar_order(vec![
                "downloads".into(),
                "videos".into(),
                "documents".into(),
                "pictures".into(),
                "desktop".into(),
            ]);
            assert_eq!(
                *first.state.place_order.borrow(),
                ["downloads", "videos", "documents", "pictures", "desktop"]
            );
            assert_eq!(
                *first.state.place_order.borrow(),
                *second.state.place_order.borrow()
            );
            let cleared = Rc::new(Cell::new(0));
            let observe = cleared.clone();
            let notice: crate::ui::settings::UpdateNoticeHandler = Rc::new(move |value| {
                assert!(value.is_none());
                observe.set(observe.get() + 1);
            });
            let anchors = [
                gtk::Box::new(gtk::Orientation::Vertical, 0),
                gtk::Box::new(gtk::Orientation::Vertical, 0),
            ];
            for anchor in &anchors {
                super::super::bind_update_notice_preferences(anchor, &manager, &notice);
            }
            assert_eq!(cleared.get(), 0);
            manager.set_checks_for_updates(true);
            assert_eq!(cleared.get(), 2);
            manager.set_release_channel(crate::services::Channel::Stable);
            assert_eq!(cleared.get(), 4);
            manager.set_checks_for_updates(false);
            assert_eq!(cleared.get(), 6);
        },
    );
}

#[test]
fn saved_browser_preferences_apply_without_settings_and_survive_view_changes() {
    gtk_test(
        "ui::window::tests::preferences::saved_browser_preferences_apply_without_settings_and_survive_view_changes",
        || {
            let directory = glib::user_config_dir().join("strata");
            std::fs::create_dir_all(&directory).expect("isolated settings directory");
            let path = directory.join("settings.toml");
            let saved = r#"
mode = "theme"
theme = "azure-glow"
folder_peeking = false
single_click_previews = false
browser_mode = "list"
browser_density = "airy"
group_by_type = true
list_file_clicks = 1
list_folder_clicks = 2
grid_file_clicks = 1
grid_folder_clicks = 1
explorer_file_clicks = 1
explorer_folder_clicks = 1
show_hidden = true
folders_first = false
sort_key = "size"
sort_direction = "descending"
auto_refresh_interval = 600
"#;
            std::fs::write(&path, saved).expect("persist non-default startup preferences");
            let manager = ThemeManager::shared();
            assert!(!manager.folder_peeking());
            assert!(!manager.single_click_previews());
            assert_eq!(manager.browser_mode(), BrowserMode::List);
            for _ in 0..2 {
                let browser = browser_for_window();
                assert_eq!(browser.view_mode(), BrowserMode::List);
                browser.assert_saved_preferences(&manager);
                browser.assert_peek_scheduling(false);
                for mode in [BrowserMode::Icons, BrowserMode::Columns, BrowserMode::List] {
                    browser.set_view_mode(mode);
                    browser.assert_saved_preferences(&manager);
                    browser.assert_peek_scheduling(false);
                }
            }
            assert_eq!(
                std::fs::read_to_string(path).expect("unchanged preferences"),
                saved
            );
        },
    );
}

#[test]
fn default_browser_preferences_allow_peeking_without_settings() {
    gtk_test(
        "ui::window::tests::preferences::default_browser_preferences_allow_peeking_without_settings",
        || {
            let manager = ThemeManager::shared();
            let browser = browser_for_window();
            assert!(manager.folder_peeking());
            browser.assert_saved_preferences(&manager);
            browser.assert_peek_scheduling(true);
            browser.set_peek_enabled(false);
            browser.assert_peek_scheduling(false);
            browser.set_peek_enabled(true);
            browser.assert_peek_scheduling(true);
        },
    );
}
