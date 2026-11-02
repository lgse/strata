// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn sidebar_folder_customization_loads_and_updates_across_windows() {
    gtk_test(
        "ui::window::tests::preferences::sidebar_prefs::sidebar_folder_customization_loads_and_updates_across_windows",
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
            crate::test_support::save_pins(&[(location.clone(), "Custom folder".into())]);
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
            let local_sidebar = crate::ui::window::sidebar::build_sidebar(
                first.content.browser.clone(),
                manager.clone(),
                true,
            );
            let sidebars = [
                first.content.sidebar.state.clone(),
                second.content.sidebar.state.clone(),
                local_sidebar.state.clone(),
            ];
            let sidebar_image = |state: &crate::ui::window::SidebarState, location: &Location| {
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

#[test]
fn window_buttons_load_and_follow_settings_across_open_and_later_windows() {
    gtk_test(
        "ui::window::tests::preferences::sidebar_prefs::window_buttons_load_and_follow_settings_across_open_and_later_windows",
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
            crate::storage::writer::settle();
            let saved = std::fs::read(settings_file()).expect("saved settings");
            for open in [&first, &second] {
                open.content.settings_button().emit_clicked();
            }
            settle();
            for page in ["keyboard", "general"] {
                for open in [&first, &second] {
                    settings_page_button(open, page).emit_clicked();
                }
                settle();
            }
            crate::storage::writer::settle();
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
            crate::storage::writer::settle();
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
fn saved_sidebar_compact_starts_railed_and_the_toggle_steps_both_windows() {
    gtk_test(
        "ui::window::tests::preferences::sidebar_prefs::saved_sidebar_compact_starts_railed_and_the_toggle_steps_both_windows",
        || {
            write_settings("sidebar_compact = true\n");
            let manager = PreferenceManager::shared();
            assert!(manager.sidebar_compact());
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            settle();
            for open in [&first, &second] {
                assert!(open.content.sidebar_visible());
                assert!(
                    open.content.sidebar.state.rail.get(),
                    "a saved compact sidebar starts as the rail in a wide window"
                );
            }
            first.content.sidebar_toggle().emit_clicked();
            settle();
            assert!(!manager.sidebar_expanded(), "the rail's next step hides");
            for open in [&first, &second] {
                wait_until_or(
                    || !open.content.sidebar_visible(),
                    || sidebar_report(&[&first, &second], &manager),
                );
            }
            second.content.sidebar_toggle().emit_clicked();
            settle();
            assert!(manager.sidebar_expanded());
            assert!(
                !manager.sidebar_compact(),
                "hidden steps to the full sidebar"
            );
            for open in [&first, &second] {
                wait_until_or(
                    || open.content.sidebar_visible(),
                    || sidebar_report(&[&first, &second], &manager),
                );
                assert!(!open.content.sidebar.state.rail.get());
            }
            first.content.sidebar_toggle().emit_clicked();
            settle();
            assert!(manager.sidebar_compact(), "full steps to the rail");
            for open in [&first, &second] {
                wait_until_or(
                    || open.content.sidebar.state.rail.get(),
                    || sidebar_report(&[&first, &second], &manager),
                );
                assert!(open.content.sidebar_visible());
            }
            crate::storage::writer::settle();
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("sidebar_compact = true"), "{saved}");
        },
    );
}

#[test]
fn narrow_window_rails_automatically_and_the_toggle_reaches_every_mode() {
    gtk_test(
        "ui::window::tests::preferences::sidebar_prefs::narrow_window_rails_automatically_and_the_toggle_reaches_every_mode",
        || {
            let manager = PreferenceManager::shared();
            manager.set_browser_mode(BrowserMode::Columns);
            let open = OpenWindow::open_with_width(700);
            let sidebar = &open.content.sidebar.state;
            wait_until_or(|| sidebar.rail.get(), || sidebar_report(&[&open], &manager));
            assert!(!manager.sidebar_compact(), "an automatic rail is not saved");

            let ctrl_b = || {
                press(
                    &open.window,
                    gtk::gdk::Key::b,
                    gtk::gdk::ModifierType::CONTROL_MASK,
                )
            };
            ctrl_b();
            wait_until_or(
                || !open.content.sidebar_visible(),
                || sidebar_report(&[&open], &manager),
            );
            assert!(!manager.sidebar_expanded());

            ctrl_b();
            assert!(
                !sidebar.rail.get(),
                "showing a hidden sidebar asks for the full sidebar"
            );
            wait_until_or(
                || open.content.sidebar_visible(),
                || sidebar_report(&[&open], &manager),
            );
            settle_for(std::time::Duration::from_millis(400));
            assert!(
                !sidebar.rail.get(),
                "the full sidebar stays while the shortage is unchanged"
            );
            assert!(!manager.sidebar_compact());

            ctrl_b();
            wait_until_or(|| sidebar.rail.get(), || sidebar_report(&[&open], &manager));
            assert!(manager.sidebar_compact(), "full steps to the rail");
            assert!(open.content.sidebar_visible());

            ctrl_b();
            wait_until_or(
                || !open.content.sidebar_visible(),
                || sidebar_report(&[&open], &manager),
            );
            assert!(manager.sidebar_compact(), "hiding keeps the compact choice");
        },
    );
}

#[test]
fn saved_sidebar_collapsed_restores_and_ctrl_b_updates_both_windows() {
    gtk_test(
        "ui::window::tests::preferences::sidebar_prefs::saved_sidebar_collapsed_restores_and_ctrl_b_updates_both_windows",
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
                wait_until_or(
                    || open.content.sidebar_visible(),
                    || sidebar_report(&[&first, &second], &manager),
                );
            }
            press(
                &second.window,
                gtk::gdk::Key::b,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(manager.sidebar_expanded());
            assert!(
                manager.sidebar_compact(),
                "the full sidebar steps to the rail"
            );
            for open in [&first, &second] {
                assert!(open.content.sidebar_toggle().is_active());
                wait_until_or(
                    || open.content.sidebar.state.rail.get(),
                    || sidebar_report(&[&first, &second], &manager),
                );
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
                wait_until_or(
                    || !open.content.sidebar_visible(),
                    || sidebar_report(&[&first, &second], &manager),
                );
            }
            crate::storage::writer::settle();
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("sidebar_expanded = false"), "{saved}");
        },
    );
}
