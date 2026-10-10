// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::*;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::preferences::PreferenceManager;
use crate::ui::tenxer_mode::UNUSED_SUBTITLE;

#[test]
fn sidebar_folder_customization_loads_and_updates_across_windows() {
    gtk_test(
        "ui::window::tests::preferences::sidebar_folder_customization_loads_and_updates_across_windows",
        || {
            use crate::assets::{self, icons};
            use crate::model::{FolderColorValue, Location};

            let directory = tempfile::tempdir().expect("pinned folder");
            let path = directory.path();
            let location = Location::local(path);
            let home = gtk::glib::home_dir();
            let downloads = home.join("Downloads");
            std::fs::create_dir_all(&downloads).expect("Downloads fixture");
            std::fs::create_dir_all(gtk::glib::user_config_dir()).expect("config fixture");
            std::fs::write(
                gtk::glib::user_config_dir().join("user-dirs.dirs"),
                "XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n",
            )
            .expect("XDG directories fixture");
            gtk::glib::reload_user_special_dirs_cache();
            assert_eq!(
                gtk::glib::user_special_dir(gtk::glib::UserDirectory::Downloads),
                Some(downloads.clone())
            );
            let cases = [
                (location.clone(), icons::FOLDER),
                (Location::local(&home), icons::HOME),
                (Location::local(&downloads), icons::DOWNLOADS),
            ];
            super::super::save_pinned_places(&[(location.clone(), "Custom folder".into())])
                .expect("save pin");
            let mut settings = String::from("[folder_colors]\n");
            for (location, _) in &cases {
                settings.push_str(&format!("\"{}\" = \"red\"\n", location.display_path()));
            }
            settings.push_str("[custom_icons]\n");
            for (location, _) in &cases {
                settings.push_str(&format!(
                    "\"{}\" = \"{}\"\n",
                    location.display_path(),
                    icons::PICTURES,
                ));
            }
            write_settings(&settings);
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let local_sidebar = super::super::sidebar::build_sidebar(
                first.content.browser.clone(),
                manager.clone(),
                true,
            );
            let sidebars = [
                first.content.sidebar.state.clone(),
                second.content.sidebar.state.clone(),
                local_sidebar.state.clone(),
            ];
            let sidebar_image = |state: &super::super::SidebarState, location: &Location| {
                let row = state
                    .place_rows
                    .borrow()
                    .iter()
                    .find(|(candidate, _)| candidate == location)
                    .expect("folder row")
                    .1
                    .clone();
                row.child()
                    .and_then(|content| content.first_child())
                    .and_downcast::<gtk::Image>()
                    .expect("folder icon")
            };
            let assert_icons = |location: &Location, expected: gtk::gdk::Texture| {
                for state in &sidebars {
                    let actual = sidebar_image(state, location)
                        .paintable()
                        .expect("rendered icon")
                        .downcast::<gtk::gdk::Texture>()
                        .expect("icon texture");
                    assert!(
                        texture_pixels(&actual) == texture_pixels(&expected),
                        "sidebar icon differs from expected folder customization: {}",
                        location.display_path()
                    );
                }
            };
            for (location, _) in &cases {
                assert_icons(
                    location,
                    assets::folder_decoration_paintable(
                        icons::PICTURES,
                        manager
                            .folder_color(location.native_path().expect("folder path"))
                            .expect("saved color")
                            .hex(),
                    )
                    .expect("decorated folder"),
                );
            }
            for state in &sidebars {
                state.set_rail(true);
            }
            for (location, fallback) in &cases {
                let path = location.native_path().expect("folder path");
                manager.set_folder_color(path, Some(FolderColorValue::Custom("#123456".into())));
                assert_icons(
                    location,
                    assets::folder_decoration_paintable(icons::PICTURES, "#123456")
                        .expect("recolored folder"),
                );
                manager.set_custom_icon(path, Some("emoji:🚀"));
                assert_icons(
                    location,
                    assets::folder_decoration_paintable("emoji:🚀", "#123456")
                        .expect("emoji folder"),
                );
                manager.clear_item_customization(path);
                assert_icons(
                    location,
                    assets::primary_icon_paintable(fallback).expect("default folder"),
                );
                manager.set_folder_color(path, Some(FolderColorValue::Custom("#123456".into())));
                assert_icons(
                    location,
                    assets::custom_colored_icon_paintable(fallback, "#123456")
                        .expect("color-only folder"),
                );
                manager.clear_item_customization(path);
            }
            for state in &sidebars {
                state.set_rail(false);
                state.rebuild();
            }
            for open in [&first, &second] {
                assert!(settings_closed(open));
            }
            for (location, _) in &cases {
                manager.set_custom_icon(
                    location.native_path().expect("folder path"),
                    Some(icons::KEY),
                );
                assert_icons(
                    location,
                    assets::folder_decoration_paintable(icons::KEY, &assets::primary_icon_color())
                        .expect("rebuilt folder decoration"),
                );
            }
            crate::ui::theme::ThemeManager::shared().select_theme("dracula");
            for (location, _) in &cases {
                assert_icons(
                    location,
                    assets::folder_decoration_paintable(icons::KEY, &assets::primary_icon_color())
                        .expect("theme-colored folder"),
                );
            }
            let row = first
                .content
                .sidebar
                .state
                .place_rows
                .borrow()
                .iter()
                .find(|(candidate, _)| candidate == &location)
                .expect("rebuilt pin")
                .1
                .clone();
            walk(row.upcast_ref(), &mut |widget| {
                if let Some(popover) = widget.downcast_ref::<gtk::Popover>() {
                    popover.popup();
                }
            });
            settle();
            let customize = label_named(row.upcast_ref(), "Customize…")
                .ancestor(gtk::Button::static_type())
                .and_downcast::<gtk::Button>()
                .expect("customize action");
            customize.emit_clicked();
            assert!(
                !class_in_shown_pane(first.content.overlay().upcast_ref(), "customize-dialog")
                    .is_empty()
            );
            label_named(first.content.overlay().upcast_ref(), "Customize Folder");
            local_sidebar.disconnect();
        },
    );
}

fn texture_pixels(texture: &gtk::gdk::Texture) -> Vec<u8> {
    let stride = texture.width() as usize * 4;
    let mut pixels = vec![0; stride * texture.height() as usize];
    texture.download(&mut pixels, stride);
    pixels
}

#[test]
fn default_chrome_stays_operable_without_a_saved_tenxer_mode() {
    gtk_test(
        "ui::window::tests::preferences::default_chrome_stays_operable_without_a_saved_tenxer_mode",
        || {
            let manager = PreferenceManager::shared();
            assert!(!manager.tenxer_mode());
            let open = OpenWindow::open();
            let _directory = load_folder(&open);
            assert!(!open.content.footer().tag_visible());
            assert_controls(&open, true);
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                open.content.browser.set_view_mode(mode);
                wait_until(|| !column_loading(&open, 0));
                assert_icon_only_tooltips(open.window.upcast_ref());
            }
            open.content.sidebar.state.set_rail(true);
            assert_icon_only_tooltips(open.window.upcast_ref());
            open.content.sidebar.state.set_rail(false);
            assert_icon_only_tooltips(open.window.upcast_ref());
            press(
                &open.window,
                gtk::gdk::Key::q,
                gtk::gdk::ModifierType::empty(),
            );
            assert!(window_listed(&open.window));
            assert!(!manager.tenxer_mode());
            assert!(!open.content.footer().tag_visible());
        },
    );
}

#[test]
fn window_buttons_load_and_follow_settings_across_open_and_later_windows() {
    gtk_test(
        "ui::window::tests::preferences::window_buttons_load_and_follow_settings_across_open_and_later_windows",
        || {
            write_settings(
                "window_show_minimize = true\nwindow_show_maximize = true\nwindow_show_close = false\n",
            );
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let assert_buttons = |open: &OpenWindow, expected: [bool; 3]| {
                for (button, visible) in [
                    open.content.minimize_button(),
                    open.content.maximize_button(),
                    open.content.close_button(),
                ]
                .into_iter()
                .zip(expected)
                {
                    assert_eq!(button.is_visible(), visible);
                }
            };
            for open in [&first, &second] {
                assert!(settings_closed(open));
                assert_buttons(open, [true, true, false]);
            }
            let saved = std::fs::read(settings_file()).expect("saved settings");
            for open in [&first, &second] {
                open.content.settings_button().emit_clicked();
            }
            settle();
            assert_eq!(
                std::fs::read(settings_file()).expect("settings unchanged"),
                saved
            );
            let titles = [
                "Show minimize button",
                "Show maximize button",
                "Show close button",
            ];
            for (index, title) in titles.into_iter().enumerate() {
                let switch = switch_named(first.content.overlay(), title);
                let initial = index != 2;
                switch.set_active(!initial);
                settle();
                let mut expected = [true, true, false];
                expected[index] = !initial;
                for open in [&first, &second] {
                    assert_buttons(open, expected);
                    assert_eq!(
                        switch_named(open.content.overlay(), title).is_active(),
                        !initial
                    );
                }
                switch_named(second.content.overlay(), title).set_active(initial);
                settle();
                for open in [&first, &second] {
                    assert_buttons(open, [true, true, false]);
                }
            }
            manager.set_window_show_minimize(false);
            manager.set_window_show_maximize(false);
            manager.set_window_show_close(true);
            let third = OpenWindow::open();
            for open in [&first, &second, &third] {
                assert_buttons(open, [false, false, true]);
            }
            let persisted: toml::Value = toml::from_str(
                &std::fs::read_to_string(settings_file()).expect("persisted preferences"),
            )
            .expect("valid settings");
            assert_eq!(persisted["window_show_minimize"].as_bool(), Some(false));
            assert_eq!(persisted["window_show_maximize"].as_bool(), Some(false));
            assert_eq!(persisted["window_show_close"].as_bool(), Some(true));
        },
    );
}

#[test]
fn saved_tenxer_mode_applies_before_settings_and_to_lazy_views() {
    gtk_test(
        "ui::window::tests::preferences::saved_tenxer_mode_applies_before_settings_and_to_lazy_views",
        || {
            write_settings("tenxer_mode = true\n");
            let manager = PreferenceManager::shared();
            assert!(manager.tenxer_mode());
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            assert!(settings_closed(&first));
            assert!(settings_closed(&second));
            assert!(first.content.footer().tag_visible());
            assert!(second.content.footer().tag_visible());
            let directory = load_folder(&first);
            second
                .content
                .browser
                .navigate_location(crate::model::Location::local(directory.path()));
            wait_until(|| !column_loading(&second, 0));
            for open in [&first, &second] {
                assert_controls(open, false);
                assert!(open.content.close_button().is_visible());
                assert!(open.content.close_button().is_sensitive());
            }
            open_child_column(&first);
            assert!(
                !controls_in_shown_pane(&first.content.browser.widget(), "Close this pane")
                    .is_empty()
            );
            assert_controls(&first, false);
            first.content.browser.set_view_mode(BrowserMode::Icons);
            wait_until(|| !column_loading(&first, 0));
            assert_controls(&first, false);
            first.content.browser.set_view_mode(BrowserMode::List);
            wait_until(|| !column_loading(&first, 0));
            assert_controls(&first, false);
            let heading = list_heading(&first.content.browser.widget(), "Name");
            assert!(heading.is_visible() && heading.is_sensitive());
            let depth = first
                .content
                .browser
                .browser()
                .active_depth()
                .expect("list depth");
            let before = first
                .content
                .browser
                .browser()
                .column_preferences(depth)
                .expect("list preferences")
                .sort_direction;
            heading.emit_clicked();
            settle_for(std::time::Duration::from_millis(50));
            let after = first
                .content
                .browser
                .browser()
                .column_preferences(depth)
                .expect("sorted list preferences")
                .sort_direction;
            assert_ne!(before, after);
            manager.set_tenxer_mode(false);
            settle();
            assert_controls(&first, true);
            assert!(!first.content.footer().tag_visible());
            drop(directory);
        },
    );
}

#[test]
fn browsing_control_and_shortcut_update_both_windows() {
    gtk_test(
        "ui::window::tests::preferences::browsing_control_and_shortcut_update_both_windows",
        || {
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let directory = load_folder(&first);
            second
                .content
                .browser
                .navigate_location(crate::model::Location::local(directory.path()));
            wait_until(|| !column_loading(&second, 0));
            press(
                &first.window,
                gtk::gdk::Key::m,
                gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
            );
            settle();
            assert!(manager.tenxer_mode());
            assert_controls(&first, false);
            assert_controls(&second, false);
            assert!(first.content.footer().tag_visible());
            assert!(second.content.footer().tag_visible());
            first.content.settings_button().emit_clicked();
            second.content.settings_button().emit_clicked();
            settle();
            let first_switch = switch_named(first.content.overlay(), "10xer mode");
            let second_switch = switch_named(second.content.overlay(), "10xer mode");
            assert!(first_switch.is_active());
            assert!(second_switch.is_active());
            second_switch.set_active(false);
            settle();
            assert!(!manager.tenxer_mode());
            assert!(!first_switch.is_active());
            assert_controls(&first, true);
            assert_controls(&second, true);
            assert!(!second.content.footer().tag_visible());
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("tenxer_mode = false"));
        },
    );
}

#[test]
fn saved_sidebar_collapsed_restores_and_ctrl_b_updates_both_windows() {
    gtk_test(
        "ui::window::tests::preferences::saved_sidebar_collapsed_restores_and_ctrl_b_updates_both_windows",
        || {
            write_settings("sidebar_expanded = false\n");
            let manager = PreferenceManager::shared();
            assert!(!manager.sidebar_expanded());
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            for open in [&first, &second] {
                assert!(!open.content.sidebar_toggle().is_active());
                assert!(!open.content.sidebar_visible());
            }
            press(
                &first.window,
                gtk::gdk::Key::b,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(manager.sidebar_expanded());
            for open in [&first, &second] {
                assert!(open.content.sidebar_toggle().is_active());
                wait_until(|| open.content.sidebar_visible());
            }
            press(
                &second.window,
                gtk::gdk::Key::b,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(!manager.sidebar_expanded());
            for open in [&first, &second] {
                assert!(!open.content.sidebar_toggle().is_active());
                wait_until(|| !open.content.sidebar_visible());
            }
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("sidebar_expanded = false"), "{saved}");
        },
    );
}

#[test]
fn toggle_leaves_tenxer_everywhere_and_shift_q_closes_only_the_current_window() {
    gtk_test(
        "ui::window::tests::preferences::toggle_leaves_tenxer_everywhere_and_shift_q_closes_only_the_current_window",
        || {
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            manager.set_tenxer_mode(true);
            settle();
            press(
                &first.window,
                gtk::gdk::Key::m,
                gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
            );
            settle();
            assert!(!manager.tenxer_mode());
            assert!(window_listed(&first.window));
            assert!(window_listed(&second.window));
            assert!(!first.content.footer().tag_visible());
            assert!(!second.content.footer().tag_visible());
            manager.set_tenxer_mode(true);
            settle();
            press(
                &first.window,
                gtk::gdk::Key::Q,
                gtk::gdk::ModifierType::SHIFT_MASK,
            );
            settle();
            assert!(!window_listed(&first.window));
            assert!(window_listed(&second.window));
            assert!(manager.tenxer_mode());
            assert!(second.content.footer().tag_visible());
        },
    );
}

#[test]
fn mode_cycles_and_closed_windows_leave_no_duplicate_commands_or_listeners() {
    gtk_test(
        "ui::window::tests::preferences::mode_cycles_and_closed_windows_leave_no_duplicate_commands_or_listeners",
        || {
            let manager = PreferenceManager::shared();
            let toggle = gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK;
            let first = OpenWindow::open();
            settle();
            let baseline = manager.listener_count();
            for _ in 0..3 {
                manager.set_tenxer_mode(true);
                settle();
                manager.set_tenxer_mode(false);
                settle();
            }
            assert_eq!(manager.listener_count(), baseline, "cycles added listeners");

            let closed = {
                let window = gtk::ApplicationWindow::builder()
                    .application(&test_application())
                    .build();
                let content = super::super::composition::WindowContent::new(&window, &manager);
                content.bind(&window, &manager);
                content.connect_cleanup(&window);
                window.present();
                settle();
                assert!(manager.listener_count() > baseline);
                window.destroy();
                window.downgrade()
            };
            settle_for(std::time::Duration::from_millis(100));
            assert_eq!(
                manager.listener_count(),
                baseline,
                "a closed window kept listeners"
            );
            assert!(
                closed.upgrade().is_none(),
                "a closed window was never released"
            );
            assert!(window_listed(&first.window));

            let third = OpenWindow::open();
            settle();
            for (window, expected) in [(&third.window, true), (&first.window, false)] {
                press(window, gtk::gdk::Key::M, toggle);
                settle();
                assert_eq!(manager.tenxer_mode(), expected, "one toggle per press");
                assert_eq!(first.content.footer().tag_visible(), expected);
                assert_eq!(third.content.footer().tag_visible(), expected);
            }
        },
    );
}

#[test]
fn browsing_preferences_stay_saved_but_unused_until_exit() {
    gtk_test(
        "ui::window::tests::preferences::browsing_preferences_stay_saved_but_unused_until_exit",
        || {
            let manager = PreferenceManager::shared();
            manager.set_type_to_search(true);
            manager.set_arrow_navigation_scoped(true);
            manager.set_columns_mirror_selection(true);
            let open = OpenWindow::open();
            let directory = load_folder(&open);
            assert!(open.content.browser.columns_mirror_selection_enabled());
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .any(|button| button.is_active()),
                "type to search filters before 10xer is enabled"
            );
            for button in filter_buttons(&open) {
                if button.is_active() {
                    button.set_active(false);
                }
            }
            open.content.settings_button().emit_clicked();
            settle();
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Include subfolders",
                "Mirror columns selection",
            ] {
                let switch = switch_named(open.content.overlay(), title);
                assert!(switch.is_active() && switch.is_sensitive());
                assert_ne!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            manager.set_tenxer_mode(true);
            settle();
            assert!(manager.type_to_search());
            assert!(manager.arrow_navigation_scoped());
            assert!(
                open.content.browser.columns_mirror_selection_enabled(),
                "10xer Columns keep saved mirroring"
            );
            assert_ne!(
                description_named(open.content.overlay(), "Mirror columns selection"),
                UNUSED_SUBTITLE
            );
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Include subfolders",
            ] {
                let switch = switch_named(open.content.overlay(), title);
                assert!(switch.is_active() && switch.is_sensitive());
                assert_eq!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            let type_to_search = switch_named(open.content.overlay(), "Type to search");
            type_to_search.set_active(false);
            settle();
            assert!(!manager.type_to_search());
            type_to_search.set_active(true);
            settle();
            assert!(manager.type_to_search());
            close_settings(&open);
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .all(|button| !button.is_active()),
                "typing does not filter while 10xer is on"
            );
            manager.set_tenxer_mode(false);
            settle();
            assert!(manager.type_to_search());
            assert!(open.content.browser.columns_mirror_selection_enabled());
            open.content.settings_button().emit_clicked();
            settle();
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Include subfolders",
                "Mirror columns selection",
            ] {
                assert_ne!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            close_settings(&open);
            open.content.browser.set_view_mode(BrowserMode::Columns);
            wait_until(|| !column_loading(&open, 0));
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .any(|button| button.is_active()),
                "type to search filters again after leaving 10xer"
            );
            drop(directory);
        },
    );
}

#[test]
fn entering_tenxer_hides_an_open_filter_and_leaving_restores_it() {
    gtk_test(
        "ui::window::tests::preferences::entering_tenxer_hides_an_open_filter_and_leaving_restores_it",
        || {
            let manager = PreferenceManager::shared();
            let open = OpenWindow::open();
            let directory = load_folder(&open);
            let filter =
                controls_in_shown_pane(&open.content.browser.widget(), "Filter this pane (Ctrl+F)")
                    .into_iter()
                    .next()
                    .expect("filter button");
            filter
                .downcast_ref::<gtk::ToggleButton>()
                .expect("filter toggle")
                .emit_clicked();
            settle();
            let revealer = revealer_for_filter(&filter);
            assert!(revealer.reveals_child());
            manager.set_tenxer_mode(true);
            settle();
            assert!(!revealer.reveals_child());
            assert_controls(&open, false);
            manager.set_tenxer_mode(false);
            settle();
            assert!(revealer.reveals_child());
            assert_controls(&open, true);
            drop(directory);
        },
    );
}

#[test]
fn ctrl_f_cannot_activate_hidden_filter_or_displace_existing_column_filter() {
    gtk_test(
        "ui::window::tests::preferences::ctrl_f_cannot_activate_hidden_filter_or_displace_existing_column_filter",
        || {
            let manager = PreferenceManager::shared();
            let open = OpenWindow::open();
            let _directory = load_folder(&open);
            let first = filter_buttons(&open)
                .into_iter()
                .next()
                .expect("first filter");
            let first_revealer = revealer_for_filter(first.upcast_ref());
            manager.set_tenxer_mode(true);
            press(
                &open.window,
                gtk::gdk::Key::f,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(!first.is_active());
            manager.set_tenxer_mode(false);
            settle();
            assert!(!first_revealer.reveals_child());

            open_child_column(&open);
            first.set_active(true);
            settle();
            assert!(first.is_active());
            manager.set_tenxer_mode(true);
            press(
                &open.window,
                gtk::gdk::Key::f,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(first.is_active(), "existing filter remains active");
            assert_eq!(
                filter_buttons(&open)
                    .iter()
                    .filter(|button| button.is_active())
                    .count(),
                1
            );
            manager.set_tenxer_mode(false);
            settle();
            assert!(first_revealer.reveals_child());
        },
    );
}

#[test]
fn saved_folder_sorts_and_icon_sizes_apply_before_settings_and_follow_changes_across_windows() {
    gtk_test(
        "ui::window::tests::preferences::saved_folder_sorts_and_icon_sizes_apply_before_settings_and_follow_changes_across_windows",
        || {
            use std::os::unix::ffi::OsStrExt;

            use crate::model::{FolderSort, Location, SortDirection, SortKey};

            let sorted = tempfile::tempdir().expect("sorted folder");
            let plain = tempfile::tempdir().expect("plain folder");
            let sorted_location = Location::local(sorted.path());
            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"size\"\ndirection = \"descending\"\nicons_size = 192\n",
                sorted.path().display()
            ));
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let sorting = |open: &OpenWindow| {
                open.content
                    .browser
                    .browser()
                    .column_preferences(0)
                    .map(|preferences| (preferences.sort_key, preferences.sort_direction))
            };
            let show = |open: &OpenWindow, location: &Location| {
                open.content.browser.navigate_location(location.clone());
                wait_until(|| {
                    open.content.browser.browser().location_at(0).as_ref() == Some(location)
                        && !column_loading(open, 0)
                });
            };

            show(&first, &sorted_location);
            show(&second, &Location::local(plain.path()));
            assert_eq!(
                sorting(&first),
                Some((SortKey::Size, SortDirection::Descending))
            );
            assert_eq!(
                sorting(&second),
                Some((SortKey::Name, SortDirection::Ascending))
            );
            show(&second, &sorted_location);
            assert_eq!(
                sorting(&second),
                Some((SortKey::Size, SortDirection::Descending))
            );

            first
                .content
                .browser
                .browser()
                .set_sort(0, SortKey::Type, SortDirection::Ascending);
            wait_until(|| sorting(&second) == Some((SortKey::Type, SortDirection::Ascending)));
            assert_eq!(
                manager.default_sort(),
                (SortKey::Name, SortDirection::Ascending)
            );
            manager.flush_folder_views();
            assert!(folder_views_file().contains("sort = \"type\""));

            manager.set_browser_mode(BrowserMode::Icons);
            wait_until(|| icons_size(&first) == Some(192) && icons_size(&second) == Some(192));
            let unremembered = plain
                .path()
                .join(std::ffi::OsStr::from_bytes(b"not-utf-8-\xff"));
            std::fs::create_dir(&unremembered).expect("folder that is not remembered");
            show(&first, &Location::local(&unremembered));
            wait_until(|| icons_size(&first) == Some(manager.icons_thumbnail_size()));
            icons_scale(&first)
                .expect("icons size slider")
                .set_value(224.0);
            icons_scale(&second)
                .expect("icons size slider")
                .set_value(128.0);
            assert_eq!(manager.icons_size_for(Some(&sorted_location)), 128);
            settle();
            assert_eq!(
                icons_size(&first),
                Some(224),
                "another folder's change keeps a size that is not remembered"
            );
            show(&first, &Location::local(plain.path()));
            wait_until(|| icons_size(&first) == Some(manager.icons_thumbnail_size()));

            manager.reset_folder_sort(&sorted_location);
            manager.set_default_icons_size(128);
            assert_eq!(manager.folder_sort(&sorted_location), FolderSort::Default);
            wait_until(|| icons_size(&first) == Some(128));
            manager.set_browser_mode(BrowserMode::Columns);
            wait_until(|| sorting(&second) == Some((SortKey::Name, SortDirection::Ascending)));
        },
    );
}

#[test]
fn folder_settings_saved_by_another_process_are_merged_instead_of_overwritten() {
    gtk_test(
        "ui::window::tests::preferences::folder_settings_saved_by_another_process_are_merged_instead_of_overwritten",
        || {
            use crate::model::{FolderSort, Location, SortDirection, SortKey};

            let here = tempfile::tempdir().expect("folder sorted here");
            let elsewhere = tempfile::tempdir().expect("folder sorted elsewhere");
            let (here, elsewhere) = (
                Location::local(here.path()),
                Location::local(elsewhere.path()),
            );
            let open = OpenWindow::open();
            open.content.browser.navigate_location(elsewhere.clone());
            wait_until(|| !column_loading(&open, 0));
            let manager = PreferenceManager::shared();
            assert_eq!(manager.folder_sort(&elsewhere), FolderSort::Default);

            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"size\"\ndirection = \"descending\"\n",
                elsewhere.display_path()
            ));
            manager.set_folder_sort(&here, SortKey::Type, SortDirection::Ascending);
            manager.flush_folder_views();

            let saved = folder_views_file();
            assert!(saved.contains(&here.display_path()), "{saved}");
            assert!(saved.contains(&elsewhere.display_path()), "{saved}");
            assert_eq!(
                manager.folder_sort(&elsewhere),
                FolderSort::Saved(SortKey::Size, SortDirection::Descending)
            );
            wait_until(|| {
                open.content
                    .browser
                    .browser()
                    .column_preferences(0)
                    .is_some_and(|preferences| preferences.sort_key == SortKey::Size)
            });

            manager.forget_folder_views();
            manager.flush_folder_views();
            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"type\"\ndirection = \"descending\"\n",
                elsewhere.display_path()
            ));
            manager.forget_folder_views();
            manager.flush_folder_views();
            assert!(
                !folder_views_file().contains("[[folder]]"),
                "forgetting also clears what another process saved"
            );
            assert_eq!(manager.folder_sort(&elsewhere), FolderSort::Default);
        },
    );
}

#[test]
fn unreadable_folder_settings_use_defaults_and_are_never_overwritten() {
    gtk_test(
        "ui::window::tests::preferences::unreadable_folder_settings_use_defaults_and_are_never_overwritten",
        || {
            use crate::model::{Location, SortDirection, SortKey};

            let unreadable = "version = 1\n[[folder]\npath = \"/broken\"\n";
            write_folder_views(unreadable);
            let open = OpenWindow::open();
            let folder = load_folder(&open);
            let browser = open.content.browser.browser();
            assert_eq!(
                browser
                    .column_preferences(0)
                    .map(|preferences| preferences.sort_key),
                Some(SortKey::Name)
            );

            browser.set_sort(0, SortKey::Size, SortDirection::Descending);
            wait_until(|| {
                browser
                    .column_preferences(0)
                    .map(|preferences| preferences.sort_key)
                    == Some(SortKey::Size)
            });
            open.content
                .browser
                .navigate_location(Location::local(folder.path()));
            wait_until(|| !column_loading(&open, 0));

            assert_eq!(
                browser
                    .column_preferences(0)
                    .map(|preferences| preferences.sort_key),
                Some(SortKey::Size),
                "changes still apply until Strata closes"
            );
            PreferenceManager::shared().flush_folder_views();
            assert_eq!(folder_views_file(), unreadable);
        },
    );
}

struct OpenWindow {
    window: gtk::ApplicationWindow,
    content: super::super::composition::WindowContent,
}

impl OpenWindow {
    fn open() -> Self {
        let preferences = PreferenceManager::shared();
        let window = gtk::ApplicationWindow::builder()
            .application(&test_application())
            .title("Strata")
            .default_width(1200)
            .default_height(760)
            .build();
        let content = super::super::composition::WindowContent::new(&window, &preferences);
        content.bind(&window, &preferences);
        window.present();
        Self { window, content }
    }
}

fn test_application() -> gtk::Application {
    if let Some(application) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
        return application;
    }
    let application = gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gtk::gio::Cancellable>)
        .expect("test application registration");
    application
}

fn write_settings(contents: &str) {
    let path = settings_file();
    std::fs::create_dir_all(path.parent().expect("settings directory"))
        .expect("settings directory");
    std::fs::write(path, contents).expect("seed settings");
}

fn settings_file() -> std::path::PathBuf {
    gtk::glib::user_config_dir().join("strata/settings.toml")
}

fn write_folder_views(contents: &str) {
    let path = crate::storage::state_directory().join("folder-views.toml");
    std::fs::create_dir_all(path.parent().expect("state directory")).expect("state directory");
    std::fs::write(path, contents).expect("seed folder settings");
}

fn folder_views_file() -> String {
    std::fs::read_to_string(crate::storage::state_directory().join("folder-views.toml"))
        .expect("folder settings file")
}

fn icons_scale(open: &OpenWindow) -> Option<gtk::Scale> {
    let mut scale = None;
    walk(open.window.upcast_ref(), &mut |widget| {
        if widget.has_css_class("icons-thumbnail-scale") {
            scale = widget.clone().downcast::<gtk::Scale>().ok();
        }
    });
    scale
}

fn icons_size(open: &OpenWindow) -> Option<i32> {
    icons_scale(open).map(|scale| scale.value().round() as i32)
}

fn load_folder(open: &OpenWindow) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("folder fixture");
    std::fs::create_dir(directory.path().join("child")).expect("child folder");
    std::fs::write(directory.path().join("a.txt"), b"a").expect("file");
    open.content
        .browser
        .navigate_location(crate::model::Location::local(directory.path()));
    wait_until(|| !column_loading(open, 0));
    directory
}

fn open_child_column(open: &OpenWindow) {
    open.content.browser.browser().select(0, 0);
    open.content.browser.activate_focused();
    wait_until(|| !column_loading(open, 1));
}

fn column_loading(open: &OpenWindow, depth: usize) -> bool {
    open.content
        .browser
        .browser()
        .column_snapshot(depth)
        .is_none_or(|column| column.loading)
}

fn assert_controls(open: &OpenWindow, operable: bool) {
    assert_eq!(
        open.content.search_button().is_visible() && open.content.search_button().is_sensitive(),
        operable
    );
    if !operable {
        open.content.search_button().emit_clicked();
        assert!(
            !open.content.search_button().has_css_class("active"),
            "hidden Search does not open"
        );
    }
    if !operable && open.content.browser.view_mode() != BrowserMode::Columns {
        return;
    }
    let mut required = vec!["Refresh (F5)"];
    if open.content.browser.view_mode() != BrowserMode::List {
        required.push("Choose sort field");
    }
    for tooltip in required {
        let controls = controls_in_shown_pane(&open.content.browser.widget(), tooltip);
        assert!(!controls.is_empty(), "{tooltip}");
        for control in controls {
            assert_eq!(
                control.is_visible() && control.is_sensitive(),
                operable,
                "{tooltip}"
            );
            if !operable {
                assert!(!control.is_sensitive(), "{tooltip}");
            }
        }
    }
    let filters = [
        "Filter this pane (Ctrl+F)",
        "Filter icons (Ctrl+F)",
        "Filter list (Ctrl+F)",
    ];
    let mut found_filter = false;
    for tooltip in filters {
        for control in controls_in_shown_pane(&open.content.browser.widget(), tooltip) {
            found_filter = true;
            assert_eq!(
                control.is_visible() && control.is_sensitive(),
                operable,
                "{tooltip}"
            );
            if !operable {
                assert!(!control.is_sensitive(), "{tooltip}");
            }
        }
    }
    assert!(found_filter, "shown pane has a filter control");
    for control in class_in_shown_pane(&open.content.browser.widget(), "tenxer-sort-direction") {
        assert_eq!(control.is_visible() && control.is_sensitive(), operable);
        if !operable {
            assert!(!control.is_sensitive());
        }
    }
    for control in controls_in_shown_pane(&open.content.browser.widget(), "Close this pane") {
        assert_eq!(control.is_visible() && control.is_sensitive(), operable);
        if !operable {
            assert!(!control.is_sensitive());
        }
    }
}

fn close_settings(open: &OpenWindow) {
    activate_named(open.content.overlay(), "Close settings");
    wait_until(|| settings_closed(open));
}

fn settings_closed(open: &OpenWindow) -> bool {
    class_in_shown_pane(open.content.overlay().upcast_ref(), "settings-dialog")
        .iter()
        .all(|widget| !widget.is_visible())
}

fn window_listed(window: &gtk::ApplicationWindow) -> bool {
    window.application().is_some_and(|application| {
        application
            .windows()
            .iter()
            .any(|candidate| candidate == window)
    })
}

fn press(
    window: &gtk::ApplicationWindow,
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    let controllers = window.observe_controllers();
    let mut handled = false;
    for index in 0..controllers.n_items() {
        if let Some(keys) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            handled |= keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
        }
    }
    handled
}

fn settle() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn settle_for(duration: std::time::Duration) {
    let main_loop = gtk::glib::MainLoop::new(None, false);
    let stop = main_loop.clone();
    gtk::glib::timeout_add_local_once(duration, move || stop.quit());
    main_loop.run();
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        settle_for(std::time::Duration::from_millis(20));
    }
}

fn filter_buttons(open: &OpenWindow) -> Vec<gtk::ToggleButton> {
    controls_in_shown_pane(&open.content.browser.widget(), "Filter this pane (Ctrl+F)")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
        .collect()
}

fn assert_icon_only_tooltips(root: &gtk::Widget) {
    walk(root, &mut |widget| {
        if !widget.is_visible() || widget.tooltip_text().is_none() {
            return;
        }
        assert!(
            widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>(),
            "non-button tooltip: {}",
            widget.type_().name()
        );
        walk(widget, &mut |child| {
            if let Some(label) = child.downcast_ref::<gtk::Label>() {
                assert!(
                    !label.is_visible() || label.text().is_empty(),
                    "labelled control has a tooltip: {}",
                    label.text()
                );
            }
        });
    });
}

fn controls_in_shown_pane(root: &gtk::Widget, tooltip: &str) -> Vec<gtk::Widget> {
    widgets_in_shown_pane(root, |widget| {
        widget.tooltip_text().as_deref() == Some(tooltip)
            || widget.downcast_ref::<gtk::Entry>().is_some_and(|entry| {
                tooltip == "Filter by name. Use * for any characters: *.png, IMG*, or IMG*.png."
                    && entry.has_css_class("column-filter-entry")
            })
    })
}

fn class_in_shown_pane(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    widgets_in_shown_pane(root, |widget| widget.has_css_class(class))
}

fn widgets_in_shown_pane(
    root: &gtk::Widget,
    matches: impl Fn(&gtk::Widget) -> bool,
) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    walk(root, &mut |widget| {
        if matches(widget) && widget.parent().is_some_and(|parent| parent.is_visible()) {
            found.push(widget.clone());
        }
    });
    found
}

fn revealer_for_filter(button: &gtk::Widget) -> gtk::Revealer {
    let mut current = Some(button.clone());
    while let Some(widget) = current {
        let mut found = None;
        walk(&widget, &mut |child| {
            if found.is_none()
                && child.has_css_class("tenxer-filter-revealer")
                && let Ok(revealer) = child.clone().downcast::<gtk::Revealer>()
            {
                found = Some(revealer);
            }
        });
        if let Some(revealer) = found {
            return revealer;
        }
        current = widget.parent();
    }
    panic!("filter revealer");
}

fn list_heading(root: &gtk::Widget, name: &str) -> gtk::Button {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_some() || !widget.has_css_class("list-heading-button") {
            return;
        }
        let mut label = None;
        walk(widget, &mut |child| {
            if label.is_none()
                && child
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == name)
            {
                label = Some(child.clone());
            }
        });
        if label.is_some() {
            found = widget.clone().downcast::<gtk::Button>().ok();
        }
    });
    found.expect("list heading")
}

fn switch_named(root: &impl gtk::prelude::IsA<gtk::Widget>, title: &str) -> gtk::Switch {
    let label = label_named(root.upcast_ref(), title);
    let mut current = label.parent();
    while let Some(widget) = current {
        let mut found = None;
        walk(&widget, &mut |child| {
            if found.is_none()
                && let Ok(switch) = child.clone().downcast::<gtk::Switch>()
            {
                found = Some(switch);
            }
        });
        if let Some(switch) = found {
            return switch;
        }
        current = widget.parent();
    }
    panic!("switch for {title}");
}

fn description_named(root: &impl gtk::prelude::IsA<gtk::Widget>, title: &str) -> String {
    let label = label_named(root.upcast_ref(), title);
    let mut current = label.parent();
    while let Some(widget) = current {
        let mut found = None;
        walk(&widget, &mut |child| {
            if found.is_none()
                && child.has_css_class("settings-option-description")
                && let Some(description) = child.downcast_ref::<gtk::Label>()
            {
                found = Some(description.text().to_string());
            }
        });
        if let Some(text) = found {
            return text;
        }
        current = widget.parent();
    }
    panic!("description for {title}");
}

fn activate_named(root: &impl gtk::prelude::IsA<gtk::Widget>, tooltip: &str) {
    let button = controls_in_shown_pane(root.upcast_ref(), tooltip)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("button {tooltip}"));
    button
        .downcast_ref::<gtk::Button>()
        .unwrap_or_else(|| panic!("button {tooltip}"))
        .emit_clicked();
}

fn label_named(root: &gtk::Widget, text: &str) -> gtk::Label {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_none()
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.text() == text
            && label.is_visible()
        {
            found = Some(label.clone());
        }
    });
    found.unwrap_or_else(|| panic!("label {text}"))
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(widget) = child {
        walk(&widget, visit);
        child = widget.next_sibling();
    }
}
