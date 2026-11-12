// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn default_chrome_stays_operable_without_a_saved_tenxer_mode() {
    gtk_test(
        "ui::window::tests::preferences::tenxer_prefs::default_chrome_stays_operable_without_a_saved_tenxer_mode",
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
fn saved_tenxer_mode_applies_before_settings_and_to_lazy_views() {
    gtk_test(
        "ui::window::tests::preferences::tenxer_prefs::saved_tenxer_mode_applies_before_settings_and_to_lazy_views",
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
        "ui::window::tests::preferences::tenxer_prefs::browsing_control_and_shortcut_update_both_windows",
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
            crate::storage::writer::settle();
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("tenxer_mode = false"));
        },
    );
}

#[test]
fn override_change_reaches_two_windows() {
    gtk_test(
        "ui::window::tests::preferences::tenxer_prefs::override_change_reaches_two_windows",
        || {
            use crate::ui::keys::{Binding, Map, bindings};
            use gtk::gdk::{Key, ModifierType};
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let tooltip = |open: &OpenWindow| {
                open.content
                    .sidebar_toggle()
                    .tooltip_text()
                    .unwrap_or_default()
                    .to_string()
            };
            let footer_button = |open: &OpenWindow| {
                let mut text = None;
                walk(open.window.upcast_ref(), &mut |widget| {
                    if let Some(label) = widget.downcast_ref::<gtk::Label>()
                        && label.text().ends_with(" Shortcuts")
                    {
                        text = Some(label.text().to_string());
                    }
                });
                text.expect("footer shortcuts button")
            };
            assert!(tooltip(&second).contains("Ctrl+B"), "{}", tooltip(&second));
            assert_eq!(footer_button(&second), "F1 Shortcuts");

            manager.set_bindings(
                Map::Default,
                bindings::SIDEBAR,
                vec![Binding::ctrl(crate::ui::keys::Key::g)],
            );
            manager.set_bindings(
                Map::Default,
                bindings::REFERENCE,
                vec![Binding::plain(crate::ui::keys::Key::F12)],
            );
            settle();
            for open in [&first, &second] {
                assert!(tooltip(open).contains("Ctrl+G"), "{}", tooltip(open));
                assert_eq!(footer_button(open), "F12 Shortcuts");
            }
            assert!(manager.sidebar_expanded() && !manager.sidebar_compact());
            press(&second.window, Key::b, ModifierType::CONTROL_MASK);
            settle();
            assert!(
                !manager.sidebar_compact(),
                "Ctrl+B no longer steps the sidebar"
            );
            press(&second.window, Key::g, ModifierType::CONTROL_MASK);
            settle();
            assert!(manager.sidebar_compact(), "Ctrl+G steps the sidebar");
            for open in [&first, &second] {
                wait_until_or(
                    || open.content.sidebar.state.rail.get(),
                    || sidebar_report(&[&first, &second], &manager),
                );
            }

            manager.set_bindings(Map::Default, bindings::SIDEBAR, Vec::new());
            settle();
            for open in [&first, &second] {
                assert!(
                    ["Show sidebar", "Hide sidebar", "Compact sidebar"]
                        .contains(&tooltip(open).as_str()),
                    "an unbound toggle names no keys: {}",
                    tooltip(open)
                );
            }
            manager.reset_binding(Map::Default, bindings::SIDEBAR);
            manager.reset_map(Map::Default);
            settle();
            for open in [&first, &second] {
                assert!(tooltip(open).contains("Ctrl+B"), "{}", tooltip(open));
                assert_eq!(footer_button(open), "F1 Shortcuts");
            }
            first.window.close();
            second.window.close();
        },
    );
}

#[test]
fn capture_commits_and_second_window_follows() {
    gtk_test(
        "ui::window::tests::preferences::tenxer_prefs::capture_commits_and_second_window_follows",
        || {
            use crate::ui::keys::{Binding, Map, bindings::RENAME};
            use gtk::gdk::{Key, ModifierType};
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let _folder = load_folder(&second);
            first.content.settings_button().emit_clicked();
            settle();
            settings_page_button(&first, "keyboard").emit_clicked();
            settle();
            let mut add = None;
            walk(first.content.overlay().upcast_ref(), &mut |widget| {
                if widget.widget_name() == "keymap-default" {
                    walk(widget, &mut |row| {
                        if row.widget_name() == "settings-search-keymap.rename" {
                            walk(row, &mut |child| {
                                if let Some(button) = child.downcast_ref::<gtk::Button>()
                                    && button.label().as_deref() == Some("Add")
                                {
                                    add = Some(button.clone());
                                }
                            });
                        }
                    });
                }
            });
            add.expect("Add on the Rename row").emit_clicked();
            settle();
            assert!(press(&first.window, Key::e, ModifierType::CONTROL_MASK));
            settle();
            assert_eq!(
                manager.keymap().bindings(Map::Default, RENAME),
                [
                    Binding::plain(crate::ui::keys::Key::F2),
                    Binding::ctrl(crate::ui::keys::Key::r),
                    Binding::ctrl(crate::ui::keys::Key::e)
                ]
            );

            let browser = second.content.browser.browser();
            let position = (0..2)
                .find(|position| {
                    browser
                        .entry_at(0, *position)
                        .is_some_and(|entry| entry.display_name == "a.txt")
                })
                .expect("file position");
            browser.set_active_column(0);
            browser.select(0, position);
            browser.focus_active();
            settle();
            press(&second.window, Key::e, ModifierType::CONTROL_MASK);
            wait_until(|| second.content.browser.rename_is_active());
            assert!(second.content.browser.cancel_rename());
            close_settings(&first);
            first.window.close();
            second.window.close();
        },
    );
}

#[test]
fn toggle_leaves_tenxer_everywhere_and_shift_q_closes_only_the_current_window() {
    gtk_test(
        "ui::window::tests::preferences::tenxer_prefs::toggle_leaves_tenxer_everywhere_and_shift_q_closes_only_the_current_window",
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
        "ui::window::tests::preferences::tenxer_prefs::mode_cycles_and_closed_windows_leave_no_duplicate_commands_or_listeners",
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
                let content = crate::ui::window::composition::WindowContent::new(&window, &manager);
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
fn entering_tenxer_hides_an_open_filter_and_leaving_restores_it() {
    gtk_test(
        "ui::window::tests::preferences::tenxer_prefs::entering_tenxer_hides_an_open_filter_and_leaving_restores_it",
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
            assert!(
                focusable_outside_the_browser(&open).grab_focus(),
                "focus leaves the column while 10xer hides the filter"
            );
            settle();
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
        "ui::window::tests::preferences::tenxer_prefs::ctrl_f_cannot_activate_hidden_filter_or_displace_existing_column_filter",
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
