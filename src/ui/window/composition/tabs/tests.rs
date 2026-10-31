// SPDX-License-Identifier: MIT

use gtk::gdk;

use super::*;
use crate::test_support::gtk_test;

mod reopen;

fn press(key: gdk::Key, modifiers: gdk::ModifierType) -> KeyPress {
    KeyPress::from_signal(key, 0, modifiers)
}

fn application() -> gtk::Application {
    let application = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gio::Cancellable>)
        .expect("test application");
    application
}

fn open() -> (gtk::ApplicationWindow, Rc<TabWindow>) {
    open_in(&application())
}

fn open_in(application: &gtk::Application) -> (gtk::ApplicationWindow, Rc<TabWindow>) {
    open_persisting(application, true)
}

fn open_persisting(
    application: &gtk::Application,
    persist: bool,
) -> (gtk::ApplicationWindow, Rc<TabWindow>) {
    let preferences = PreferenceManager::shared();
    preferences.set_tenxer_mode(false);
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .default_width(1000)
        .default_height(700)
        .build();
    let tabs = TabWindow::new(&window, &preferences, persist);
    window.present();
    (window, tabs)
}

fn wait_until(condition: impl Fn() -> bool) {
    wait_until_or(condition, || "tab state did not settle".to_owned());
}

fn wait_until_or(condition: impl Fn() -> bool, failure: impl Fn() -> String) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "{}", failure());
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn load(browser: &BrowserView, location: Location) {
    browser.navigate_location(location);
    wait_until(|| {
        browser
            .browser()
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
}

fn button_with_label(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(button) = button_with_label(&widget, label) {
            return Some(button);
        }
    }
    None
}

fn location_action_value(location: &Location) -> glib::Variant {
    crate::adapters::gio_file_for_location(location)
        .uri()
        .to_variant()
}

fn find_widget(
    widget: &gtk::Widget,
    matches: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find_widget(&widget, matches) {
            return Some(found);
        }
    }
    None
}

fn folder_click(browser: &BrowserView, name: &str) -> gtk::GestureClick {
    let label = find_widget(&browser.widget(), &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == name)
    })
    .expect("folder label");
    let mut row = label;
    while !row.has_css_class("file-row") {
        row = row.parent().expect("folder row");
    }
    let controllers = row.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == 1)
        .expect("folder click gesture")
}

fn sibling_fixture(
    application: &gtk::Application,
) -> (
    tempfile::TempDir,
    gtk::ApplicationWindow,
    Rc<TabWindow>,
    BrowserView,
    gtk::Label,
) {
    let root = tempfile::tempdir().expect("sibling fixture");
    let parent = root.path().join("parent");
    std::fs::create_dir_all(parent.join("alpha")).expect("first sibling");
    std::fs::create_dir_all(parent.join("beta")).expect("second sibling");
    let (window, tabs) = open_in(application);
    tabs.preferences
        .set_browser_mode(crate::ui::browser_modes::BrowserMode::Columns);
    tabs.preferences.set_columns_mirror_selection(false);
    load(&tabs.active_browser(), Location::local(&parent));
    tabs.new_tab();
    let view = tabs.active_browser();
    wait_until(|| {
        view.browser()
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    view.record_pointer_hover((1.0, 1.0), Some(0));
    view.browser().activate(0, 0);
    wait_until(|| {
        view.browser()
            .column_snapshot(1)
            .is_some_and(|column| !column.loading)
    });
    let title = find_widget(tabs.strip.widget().upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == "alpha")
    })
    .expect("active tab title")
    .downcast::<gtk::Label>()
    .expect("label");
    (root, window, tabs, view, title)
}

#[test]
fn browser_windows_keep_gtk_from_adding_a_default_title_bar() {
    gtk_test(
        "ui::window::composition::tabs::tests::browser_windows_keep_gtk_from_adding_a_default_title_bar",
        || {
            let (window, tabs) = open();
            crate::ui::decoration::assert_default_decoration_absent(&window);
            tabs.new_tab();
            crate::ui::decoration::assert_default_decoration_absent(&window);
            window.destroy();
        },
    );
}

#[test]
fn saved_window_buttons_survive_tab_chrome_refreshes() {
    gtk_test(
        "ui::window::composition::tabs::tests::saved_window_buttons_survive_tab_chrome_refreshes",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let (window, tabs) = open();
            let assert_buttons = |expected: [bool; 3]| {
                let tab = tabs.active_tab();
                let header = &tab.content.header;
                for (button, visible) in [&header.minimize, &header.maximize, &header.close]
                    .into_iter()
                    .zip(expected)
                {
                    assert_eq!(button.is_visible(), visible);
                }
            };
            assert_buttons([true, true, false]);
            tabs.new_tab();
            assert_buttons([true, true, false]);
            tabs.select(1);
            assert_buttons([true, true, false]);
            tabs.preferences.set_window_show_close(true);
            tabs.preferences.set_window_show_minimize(false);
            tabs.preferences.set_window_show_maximize(false);
            assert_buttons([false, false, true]);
            tabs.select(2);
            assert_buttons([false, false, true]);
            tabs.preferences.set_window_show_close(false);
            tabs.close(2);
            assert_buttons([false, false, false]);
            tabs.new_tab();
            assert_buttons([false, false, false]);
            window.destroy();
        },
    );
}

#[test]
fn folder_click_titles_skip_parent_focus_and_keyboard_preview_stays_parent_scoped() {
    gtk_test(
        "ui::window::composition::tabs::tests::folder_click_titles_skip_parent_focus_and_keyboard_preview_stays_parent_scoped",
        || {
            let (root, window, tabs, view, title) = sibling_fixture(&application());
            let window_title = || window.title().map(|title| title.to_string());
            assert_eq!(window_title().as_deref(), Some("alpha — Strata"));
            let changes = Rc::new(RefCell::new(Vec::new()));
            let observed = changes.clone();
            title.connect_notify_local(Some("label"), move |title, _| {
                observed.borrow_mut().push(title.text().to_string());
            });
            let click = folder_click(&view, "beta");
            click.emit_by_name::<()>("pressed", &[&1i32, &1f64, &1f64]);
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(root.path().join("parent")))
            );
            let painted = Rc::new(Cell::new(false));
            let signal = painted.clone();
            let clock = window.frame_clock().expect("window frame clock");
            let handler = clock.connect_after_paint(move |_| signal.set(true));
            window.queue_draw();
            wait_until(|| painted.get());
            clock.disconnect(handler);
            assert_eq!(
                title.text(),
                "alpha",
                "holding a click must not publish the parent title"
            );
            assert_eq!(window_title().as_deref(), Some("alpha — Strata"));
            click.emit_by_name::<()>("released", &[&1i32, &1f64, &1f64]);
            wait_until(|| title.text() == "beta");
            assert_eq!(&*changes.borrow(), &["beta".to_string()]);
            assert_eq!(window_title().as_deref(), Some("beta — Strata"));
            assert_eq!(view.browser().active_depth(), Some(1));
            assert!(view.browser().selected_entries().is_empty());

            view.keyboard_navigation();
            view.browser().focus_parent();
            assert_eq!(title.text(), "parent");
            view.browser().select(0, 0);
            view.browser()
                .show_child(0, Location::local(root.path().join("parent/alpha")));
            assert_eq!(
                title.text(),
                "parent",
                "a child preview must not rename the active parent"
            );
            view.browser().activate_focused();
            assert_eq!(title.text(), "alpha");
            tabs.select(1);
            assert_eq!(
                tabs.active_browser().browser().active_location(),
                Some(Location::local(root.path().join("parent")))
            );
            assert_eq!(window_title().as_deref(), Some("parent — Strata"));
            tabs.select(2);
            assert_eq!(window_title().as_deref(), Some("alpha — Strata"));
            window.destroy();
        },
    );
}

#[test]
fn aborted_folder_clicks_release_the_title_hold() {
    gtk_test(
        "ui::window::composition::tabs::tests::aborted_folder_clicks_release_the_title_hold",
        || {
            let application = application();
            for outcome in ["cancel", "moved", "missing", "keyboard", "unmap"] {
                let (root, window, tabs, view, title) = sibling_fixture(&application);
                let click = folder_click(&view, "beta");
                click.emit_by_name::<()>("pressed", &[&1i32, &1f64, &1f64]);
                assert_eq!(title.text(), "alpha", "{outcome}: pending click");
                match outcome {
                    "cancel" => click.emit_by_name::<()>("cancel", &[&None::<gdk::EventSequence>]),
                    "moved" => click.emit_by_name::<()>("released", &[&1i32, &100f64, &100f64]),
                    "missing" => {
                        std::fs::remove_dir(root.path().join("parent/beta"))
                            .expect("remove pending destination");
                        click.emit_by_name::<()>("released", &[&1i32, &1f64, &1f64]);
                    }
                    "keyboard" => {
                        view.keyboard_navigation();
                        view.browser().focus_parent();
                        click.emit_by_name::<()>("released", &[&1i32, &1f64, &1f64]);
                    }
                    "unmap" => tabs.select(1),
                    _ => unreachable!(),
                }
                wait_until(|| title.text() == "parent");
                assert_eq!(
                    view.browser().active_location(),
                    Some(Location::local(root.path().join("parent"))),
                    "{outcome}: active location"
                );
                window.destroy();
            }
        },
    );
}

#[test]
fn tabs_preserve_locations_and_route_actions_to_the_active_browser() {
    gtk_test(
        "ui::window::composition::tabs::tests::tabs_preserve_locations_and_route_actions_to_the_active_browser",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            std::fs::create_dir(root.path().join("other")).expect("second directory");
            std::fs::write(root.path().join("one.txt"), "one").expect("first file");
            let (window, tabs) = open();
            let first = tabs.active_browser();
            load(&first, Location::local(root.path()));
            first.browser().select(0, 1);
            let selected: Vec<_> = first
                .browser()
                .selected_entries()
                .into_iter()
                .map(|entry| entry.location)
                .collect();
            tabs.new_tab();
            let second_id = tabs.active.get();
            let second = tabs.active_browser();
            load(&second, Location::local(root.path().join("other")));
            tabs.select(1);
            assert_eq!(
                first.browser().active_location(),
                Some(Location::local(root.path()))
            );
            assert_eq!(
                first
                    .browser()
                    .selected_entries()
                    .into_iter()
                    .map(|entry| entry.location)
                    .collect::<Vec<_>>(),
                selected
            );
            std::fs::write(root.path().join("other/two.txt"), "two").expect("refresh fixture");
            tabs.select(second_id);
            gio::prelude::ActionGroupExt::activate_action(&window, "refresh", None);
            wait_until(|| {
                second
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading && column.count == 1)
            });
            assert_eq!(
                first.browser().active_location(),
                Some(Location::local(root.path()))
            );
            window.destroy();
        },
    );
}

#[test]
fn returning_to_a_tab_left_by_a_tab_button_focuses_its_listing() {
    gtk_test(
        "ui::window::composition::tabs::tests::returning_to_a_tab_left_by_a_tab_button_focuses_its_listing",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            std::fs::write(root.path().join("one.txt"), "one").expect("file");
            let (window, tabs) = open();
            let first_id = tabs.active.get();
            let first = tabs.active_browser();
            load(&first, Location::local(root.path()));
            tabs.new_tab();
            let second_id = tabs.active.get();
            load(&tabs.active_browser(), Location::local(root.path()));
            tabs.select(first_id);
            wait_until(|| first.item_view_has_focus());

            // A click focuses the tab button before `clicked` selects its tab.
            let button = find_widget(tabs.strip.widget().upcast_ref(), &|widget| {
                widget.has_css_class("tab-select")
            })
            .expect("a tab button");
            assert!(button.grab_focus(), "tab buttons take focus on click");
            tabs.select(second_id);
            tabs.select(first_id);

            wait_until(|| first.item_view_has_focus());
            window.destroy();
        },
    );
}

#[test]
fn returning_to_a_tab_restores_focus_in_its_sidebar() {
    gtk_test(
        "ui::window::composition::tabs::tests::returning_to_a_tab_restores_focus_in_its_sidebar",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            let (window, tabs) = open();
            let first_id = tabs.active.get();
            load(&tabs.active_browser(), Location::local(root.path()));
            let sidebar = tabs.active_tab().content.sidebar.widget.clone();
            tabs.new_tab();
            let second_id = tabs.active.get();
            tabs.select(first_id);

            let place = || {
                find_widget(&sidebar, &|widget| {
                    widget.is::<gtk::Button>() && widget.is_mapped()
                })
            };
            wait_until(|| place().is_some());
            let row = place().expect("a sidebar place");
            assert!(row.grab_focus(), "sidebar rows take focus");
            tabs.select(second_id);
            tabs.select(first_id);

            assert_eq!(gtk::prelude::RootExt::focus(&window), Some(row));
            window.destroy();
        },
    );
}

#[test]
fn tab_shortcuts_work_in_both_modes_and_follow_reordering() {
    gtk_test(
        "ui::window::composition::tabs::tests::tab_shortcuts_work_in_both_modes_and_follow_reordering",
        || {
            use gdk::{Key, ModifierType as M};
            let (window, tabs) = open();
            let root = tempfile::tempdir().expect("tab fixture");
            load(&tabs.active_browser(), Location::local(root.path()));
            for tenxer in [false, true] {
                tabs.preferences.set_tenxer_mode(tenxer);
                for key in [Key::Page_Up, Key::Page_Down] {
                    for modifiers in [M::CONTROL_MASK, M::CONTROL_MASK | M::SHIFT_MASK] {
                        assert!(is_tab_shortcut(&press(key, modifiers)));
                        assert_eq!(
                            tabs.handle_key(&press(key, modifiers)),
                            glib::Propagation::Stop
                        );
                        assert_eq!(tabs.active.get(), 1);
                    }
                }
                assert_eq!(
                    tabs.handle_key(&press(Key::t, M::CONTROL_MASK)),
                    glib::Propagation::Stop
                );
                let new_id = tabs.active.get();
                tabs.handle_key(&press(Key::exclam, M::CONTROL_MASK | M::SHIFT_MASK));
                assert_eq!(tabs.active.get(), 1);
                tabs.reorder(new_id, 1);
                tabs.handle_key(&press(Key::exclam, M::CONTROL_MASK | M::SHIFT_MASK));
                assert_eq!(tabs.active.get(), new_id);
                // Swiss German types + with Shift on the 1 key: that is Ctrl++.
                let swiss_plus =
                    KeyPress::from_signal(Key::plus, 10, M::CONTROL_MASK | M::SHIFT_MASK);
                assert!(!is_tab_shortcut(&swiss_plus));
                assert_eq!(tabs.handle_key(&swiss_plus), glib::Propagation::Proceed);
                assert_eq!(tabs.active.get(), new_id);
                tabs.handle_key(&press(Key::Tab, M::CONTROL_MASK));
                assert_eq!(tabs.active.get(), 1);
                tabs.handle_key(&press(Key::ISO_Left_Tab, M::CONTROL_MASK | M::SHIFT_MASK));
                assert_eq!(tabs.active.get(), new_id);
                tabs.new_tab();
                let last_id = tabs.active.get();
                tabs.select(new_id);
                for (key, expected) in [
                    (Key::Page_Up, last_id),
                    (Key::Page_Up, 1),
                    (Key::Page_Down, last_id),
                    (Key::Page_Down, new_id),
                    (Key::KP_Page_Up, last_id),
                    (Key::KP_Page_Up, 1),
                    (Key::KP_Page_Down, last_id),
                    (Key::KP_Page_Down, new_id),
                ] {
                    let press = press(key, M::CONTROL_MASK | M::LOCK_MASK);
                    assert!(is_tab_shortcut(&press));
                    assert_eq!(tabs.handle_key(&press), glib::Propagation::Stop);
                    assert_eq!(tabs.active.get(), expected, "{key:?}, 10xer={tenxer}");
                }
                for key in [
                    Key::Page_Up,
                    Key::Page_Down,
                    Key::KP_Page_Up,
                    Key::KP_Page_Down,
                ] {
                    for modifiers in [
                        M::empty(),
                        M::SUPER_MASK,
                        M::SHIFT_MASK,
                        M::CONTROL_MASK | M::ALT_MASK,
                        M::CONTROL_MASK | M::SUPER_MASK,
                    ] {
                        assert!(!is_tab_shortcut(&press(key, modifiers)));
                        assert_eq!(
                            tabs.handle_key(&press(key, modifiers)),
                            glib::Propagation::Proceed
                        );
                        assert_eq!(tabs.active.get(), new_id);
                    }
                    // Meta and Hyper are not shortcut modifiers.
                    for extra in [M::META_MASK, M::HYPER_MASK] {
                        assert!(is_tab_shortcut(&press(key, M::CONTROL_MASK | extra)));
                    }
                }
                tabs.close(last_id);
                tabs.handle_key(&press(Key::w, M::CONTROL_MASK));
                assert_eq!(tabs.active.get(), 1);
            }
            window.destroy();
        },
    );
}

#[test]
fn tab_reorder_shortcuts_keep_active_context_and_stop_at_edges() {
    gtk_test(
        "ui::window::composition::tabs::tests::tab_reorder_shortcuts_keep_active_context_and_stop_at_edges",
        || {
            use gdk::{Key, ModifierType as M};
            let root = tempfile::tempdir().expect("tab fixture");
            std::fs::write(root.path().join("one.txt"), "one").expect("selected file");
            let (window, tabs) = open();
            load(&tabs.active_browser(), Location::local(root.path()));
            tabs.new_tab();
            let middle = tabs.active.get();
            let browser = tabs.active_browser();
            load(&browser, Location::local(root.path()));
            browser.browser().select(0, 0);
            let selected: Vec<_> = browser.browser().selected_entries();
            tabs.new_tab();
            let last = tabs.active.get();
            tabs.select(middle);
            for tenxer in [false, true] {
                tabs.preferences.set_tenxer_mode(tenxer);
                for (key, expected) in [
                    (Key::Page_Up, vec![middle, 1, last]),
                    (Key::Page_Up, vec![middle, 1, last]),
                    (Key::Page_Down, vec![1, middle, last]),
                    (Key::Page_Down, vec![1, last, middle]),
                    (Key::Page_Down, vec![1, last, middle]),
                    (Key::KP_Page_Up, vec![1, middle, last]),
                    (Key::KP_Page_Up, vec![middle, 1, last]),
                    (Key::KP_Page_Up, vec![middle, 1, last]),
                    (Key::KP_Page_Down, vec![1, middle, last]),
                    (Key::KP_Page_Down, vec![1, last, middle]),
                    (Key::KP_Page_Down, vec![1, last, middle]),
                    (Key::KP_Page_Up, vec![1, middle, last]),
                ] {
                    let press = press(key, M::CONTROL_MASK | M::SHIFT_MASK | M::LOCK_MASK);
                    assert!(is_tab_shortcut(&press));
                    assert_eq!(tabs.handle_key(&press), glib::Propagation::Stop);
                    assert_eq!(tabs.active.get(), middle);
                    assert_eq!(
                        tabs.tabs
                            .borrow()
                            .iter()
                            .map(|tab| tab.id)
                            .collect::<Vec<_>>(),
                        expected,
                        "{key:?}, 10xer={tenxer}"
                    );
                    assert_eq!(
                        browser.browser().active_location(),
                        Some(Location::local(root.path()))
                    );
                    assert_eq!(browser.browser().selected_entries(), selected);
                }
                tabs.reorder(last, middle);
                for key in [
                    Key::Page_Up,
                    Key::Page_Down,
                    Key::KP_Page_Up,
                    Key::KP_Page_Down,
                ] {
                    for extra in [M::META_MASK, M::HYPER_MASK] {
                        let press = press(key, M::CONTROL_MASK | M::SHIFT_MASK | extra);
                        assert!(is_tab_shortcut(&press));
                    }
                    for extra in [M::ALT_MASK, M::SUPER_MASK] {
                        let press = press(key, M::CONTROL_MASK | M::SHIFT_MASK | extra);
                        assert!(!is_tab_shortcut(&press));
                        assert_eq!(tabs.handle_key(&press), glib::Propagation::Proceed);
                        assert_eq!(tabs.active.get(), middle);
                        assert_eq!(
                            tabs.tabs
                                .borrow()
                                .iter()
                                .map(|tab| tab.id)
                                .collect::<Vec<_>>(),
                            vec![1, last, middle]
                        );
                    }
                }
                tabs.reorder(last, middle);
            }
            window.destroy();
        },
    );
}

#[test]
fn closing_tabs_releases_observers_and_keeps_other_contexts_alive() {
    gtk_test(
        "ui::window::composition::tabs::tests::closing_tabs_releases_observers_and_keeps_other_contexts_alive",
        || {
            let (window, tabs) = open();
            let root = tempfile::tempdir().expect("tab fixture");
            load(&tabs.active_browser(), Location::local(root.path()));
            tabs.new_tab();
            let closed = tabs.active_browser().downgrade();
            let id = tabs.active.get();
            tabs.close(id);
            wait_until(|| closed.upgrade().is_none());
            assert_eq!(tabs.active.get(), 1);
            assert_eq!(
                tabs.active_browser().browser().active_location(),
                Some(Location::local(root.path()))
            );
            tabs.new_tab();
            let active = tabs.active.get();
            tabs.close(1);
            assert_eq!(tabs.active.get(), active);
            assert_eq!(tabs.tabs.borrow().len(), 1);
            window.destroy();
        },
    );
}

#[test]
fn hidden_tabs_and_new_tabs_apply_live_browsing_preferences() {
    gtk_test(
        "ui::window::composition::tabs::tests::hidden_tabs_and_new_tabs_apply_live_browsing_preferences",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            std::fs::write(root.path().join("visible.txt"), "visible").expect("visible file");
            std::fs::write(root.path().join(".hidden.txt"), "hidden").expect("hidden file");
            let (window, tabs) = open();
            let mut initial = tabs.preferences.sort_preferences();
            initial.show_hidden = false;
            tabs.preferences.set_sort_preferences(initial);
            let first = tabs.active_browser();
            load(&first, Location::local(root.path()));
            tabs.new_tab();
            let second = tabs.active_browser();
            wait_until(|| {
                second
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            assert_eq!(
                first
                    .browser()
                    .column_entry_counts(0)
                    .expect("first tab loaded")
                    .total,
                1
            );
            assert_eq!(
                second
                    .browser()
                    .column_entry_counts(0)
                    .expect("second tab loaded")
                    .total,
                1
            );
            let mut preferences = tabs.preferences.sort_preferences();
            preferences.show_hidden = true;
            tabs.preferences.set_sort_preferences(preferences);
            wait_until(|| {
                [first.clone(), second.clone()].iter().all(|view| {
                    view.browser()
                        .column_entry_counts(0)
                        .is_some_and(|counts| counts.total == 2)
                })
            });
            tabs.new_tab();
            wait_until(|| {
                tabs.active_browser()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|counts| counts.total == 2)
            });
            tabs.select(1);
            assert_eq!(
                tabs.active_browser()
                    .browser()
                    .column_entry_counts(0)
                    .expect("first tab retained")
                    .total,
                2
            );
            window.destroy();
        },
    );
}

fn freshness(view: &BrowserView) -> (Presence, bool) {
    let input = view.browser().freshness_input();
    (input.presence, input.power_saver)
}

#[test]
fn only_the_active_tab_checks() {
    gtk_test(
        "ui::window::composition::tabs::tests::only_the_active_tab_checks",
        || {
            let (window, tabs) = open();
            let first_id = tabs.active.get();
            let first = tabs.active_browser();
            tabs.new_tab();
            let second = tabs.active_browser();
            tabs.set_presence(Presence::Focused, false);
            let with_the_second_active = (freshness(&first).0, freshness(&second).0);
            tabs.select(first_id);
            let with_the_first_active = (freshness(&first).0, freshness(&second).0);
            tabs.new_tab();
            let third = tabs.active_browser();

            assert_eq!(
                with_the_second_active,
                (Presence::Hidden, Presence::Focused)
            );
            assert_eq!(with_the_first_active, (Presence::Focused, Presence::Hidden));
            assert_eq!(
                [&first, &second, &third].map(|view| freshness(view).0),
                [Presence::Hidden, Presence::Hidden, Presence::Focused]
            );
            window.destroy();
        },
    );
}

#[test]
fn presence_reaches_the_browser_of_the_active_tab() {
    gtk_test(
        "ui::window::composition::tabs::tests::presence_reaches_the_browser_of_the_active_tab",
        || {
            let (window, tabs) = open();
            let first = tabs.active_browser();
            tabs.new_tab();
            let second = tabs.active_browser();
            let shown = freshness(&second).0;
            window.set_visible(false);
            let hidden = (freshness(&first).0, freshness(&second).0);
            window.present();
            let shown_again = freshness(&second).0;
            tabs.set_presence(Presence::Visible, true);

            assert_ne!(shown, Presence::Hidden);
            assert_eq!(hidden, (Presence::Hidden, Presence::Hidden));
            assert_ne!(shown_again, Presence::Hidden);
            assert_eq!(
                (freshness(&first), freshness(&second)),
                ((Presence::Hidden, true), (Presence::Visible, true))
            );
            window.destroy();
        },
    );
}

#[test]
fn operations_in_inactive_tabs_prevent_tab_and_window_closure() {
    gtk_test(
        "ui::window::composition::tabs::tests::operations_in_inactive_tabs_prevent_tab_and_window_closure",
        || {
            use crate::{
                services::{PasteItem, TransferConflict},
                test_support::operations::HeldOperations,
            };
            let (window, tabs) = open();
            let root = tempfile::tempdir().expect("tab fixture");
            let browser = tabs.active_browser().browser();
            load(&tabs.active_browser(), Location::local(root.path()));
            let operations = Rc::new(HeldOperations::default());
            browser.set_operation_provider(operations.clone());
            browser.transfer(
                Location::local(root.path().join("destination")),
                vec![PasteItem {
                    source: Location::local(root.path().join("source")),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                }],
                false,
                false,
            );
            let operation = browser.last_started_operation().expect("transfer started");
            assert!(browser.has_background_operations());
            tabs.new_tab();
            assert_eq!(tabs.tabs.borrow().len(), 2);
            tabs.close(1);
            assert_eq!(tabs.tabs.borrow().len(), 2);
            assert!(tabs.closed_tabs.borrow().is_empty());
            assert!(!operations.cancelled(operation));
            window.close();
            assert!(window.is_visible());
            assert!(!operations.cancelled(operation));
            let count = tabs.tabs.borrow().len();
            tabs.new_tab();
            assert_eq!(
                tabs.tabs.borrow().len(),
                count,
                "modal input cannot create a hidden tab"
            );
            let active = tabs.active.get();
            for key in [
                gdk::Key::Page_Up,
                gdk::Key::Page_Down,
                gdk::Key::KP_Page_Up,
                gdk::Key::KP_Page_Down,
            ] {
                for modifiers in [
                    gdk::ModifierType::CONTROL_MASK,
                    gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
                ] {
                    assert_eq!(
                        tabs.handle_key(&press(key, modifiers)),
                        glib::Propagation::Proceed
                    );
                    assert_eq!(tabs.active.get(), active, "modal input cannot switch tabs");
                    assert_eq!(
                        tabs.tabs
                            .borrow()
                            .iter()
                            .map(|tab| tab.id)
                            .collect::<Vec<_>>(),
                        vec![1, active],
                        "modal input cannot reorder tabs"
                    );
                }
            }
            window.destroy();
            assert!(operations.cancelled(operation));
        },
    );
}

fn tab_locations(tabs: &TabWindow) -> Vec<Option<Location>> {
    tabs.tabs
        .borrow()
        .iter()
        .map(|tab| tab.content.browser.browser().active_location())
        .collect()
}

fn saved_session() -> Option<crate::ui::tabs_session::RestoredSession> {
    crate::ui::tabs_session::load_restorable()
}

#[test]
fn plain_launch_without_saved_session_keeps_single_tab() {
    gtk_test(
        "ui::window::composition::tabs::tests::plain_launch_without_saved_session_keeps_single_tab",
        || {
            let (_window, tabs) = open();
            assert!(!tabs.try_restore());
            assert_eq!(tabs.tabs.borrow().len(), 1);
            assert!(saved_session().is_none());
        },
    );
}

#[test]
fn saved_session_restores_tabs_in_order_with_active_tab() {
    gtk_test(
        "ui::window::composition::tabs::tests::saved_session_restores_tabs_in_order_with_active_tab",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            let first = root.path().join("first");
            let second = root.path().join("second");
            let third = root.path().join("third");
            for directory in [&first, &second, &third] {
                std::fs::create_dir_all(directory).expect("session directory");
            }
            crate::ui::tabs_session::save(
                &[
                    Location::local(root.path().join("gone")),
                    Location::local(&first),
                    Location::local(&second),
                    Location::local(&third),
                ],
                3,
            );
            crate::storage::writer::settle();
            let (window, tabs) = open();
            assert!(tabs.try_restore());
            let expected = vec![
                Some(Location::local(&first)),
                Some(Location::local(&second)),
                Some(Location::local(&third)),
            ];
            wait_until(|| tab_locations(&tabs) == expected);
            assert_eq!(tabs.tabs.borrow().len(), 3);
            assert_eq!(tabs.active.get(), tabs.tabs.borrow()[2].id);
            window.destroy();
        },
    );
}

/// Restores three tabs on folders holding a subfolder and a file, so the
/// cursor lands on a folder whose column mirrors open, as on a real launch.
fn restore_three_tabs(root: &std::path::Path) -> (gtk::ApplicationWindow, Rc<TabWindow>) {
    let folders: Vec<_> = ["first", "second", "third"]
        .into_iter()
        .map(|name| root.join(name))
        .collect();
    for folder in &folders {
        std::fs::create_dir_all(folder.join("sub")).expect("session directory");
        std::fs::write(folder.join("file.txt"), "file").expect("session file");
    }
    crate::ui::tabs_session::save(&folders.iter().map(Location::local).collect::<Vec<_>>(), 2);
    crate::storage::writer::settle();
    let (window, tabs) = open();
    assert!(tabs.try_restore());
    let expected: Vec<_> = folders
        .iter()
        .map(|folder| Some(Location::local(folder)))
        .collect();
    wait_until(|| tab_locations(&tabs) == expected);
    (window, tabs)
}

fn focused_widget(window: &gtk::ApplicationWindow) -> String {
    gtk::prelude::RootExt::focus(window).map_or_else(
        || "nothing".to_owned(),
        |widget| format!("{} {:?}", widget.type_(), widget.css_classes()),
    )
}

#[test]
fn a_restored_session_opens_with_focus_in_the_listing() {
    gtk_test(
        "ui::window::composition::tabs::tests::a_restored_session_opens_with_focus_in_the_listing",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            let (window, tabs) = restore_three_tabs(root.path());
            let active = tabs.active_browser();

            wait_until_or(
                || active.item_view_has_focus(),
                || format!("focus after restoring: {}", focused_widget(&window)),
            );
            window.destroy();
        },
    );
}

#[test]
fn a_restored_tab_shown_for_the_first_time_focuses_its_listing() {
    gtk_test(
        "ui::window::composition::tabs::tests::a_restored_tab_shown_for_the_first_time_focuses_its_listing",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            let (window, tabs) = restore_three_tabs(root.path());
            let ids: Vec<_> = tabs.tabs.borrow().iter().map(|tab| tab.id).collect();

            // Ctrl+Shift+1 and Ctrl+Shift+2: tabs restored but never shown.
            for id in &ids[..2] {
                tabs.select(*id);
                let browser = tabs.active_browser();
                wait_until_or(
                    || browser.item_view_has_focus(),
                    || format!("focus after showing tab {id}: {}", focused_widget(&window)),
                );
            }
            window.destroy();
        },
    );
}

#[test]
fn restored_tabs_open_once_a_worker_checks_their_folders() {
    gtk_test(
        "ui::window::composition::tabs::tests::restored_tabs_open_once_a_worker_checks_their_folders",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};

            let root = tempfile::tempdir().expect("tab fixture");
            let parked = root.path().join("parked");
            let kept = root.path().join("kept");
            let unmounted = root.path().join("nas");
            for directory in [&parked, &kept, &unmounted] {
                std::fs::create_dir_all(directory).expect("session directory");
            }
            let offline = unmounted.join("Documents");
            std::fs::write(root.path().join("file"), b"content").expect("session file");
            let gate = crate::test_support::Gate::default();
            let _scope = FsQuirks::install(
                root.path(),
                Quirks {
                    before: vec![(FsOp::FolderStamp, "parked", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            crate::ui::tabs_session::save(
                &[
                    Location::local(&parked),
                    Location::local(root.path().join("gone")),
                    Location::local(&kept),
                    Location::local(root.path().join("file")),
                    Location::local(&offline),
                ],
                2,
            );
            crate::storage::writer::settle();
            let (window, tabs) = open();

            assert!(tabs.try_restore());
            assert_eq!(
                tabs.tabs.borrow().len(),
                5,
                "every saved tab exists at once"
            );
            wait_until(|| gate.reached());
            wait_until(|| {
                tab_locations(&tabs)
                    == vec![
                        None,
                        Some(Location::local(&kept)),
                        Some(Location::local(&offline)),
                    ]
            });
            assert_eq!(tabs.active.get(), tabs.tabs.borrow()[1].id);
            crate::storage::writer::settle();
            assert_eq!(
                saved_session().map(|session| session.tabs),
                Some(vec![
                    Location::local(&parked),
                    Location::local(&kept),
                    Location::local(&offline)
                ]),
                "a tab still being checked, or missing from an empty mount point, stays"
            );

            gate.release();
            wait_until(|| {
                tab_locations(&tabs)
                    == vec![
                        Some(Location::local(&parked)),
                        Some(Location::local(&kept)),
                        Some(Location::local(&offline)),
                    ]
            });
            window.destroy();
        },
    );
}

#[test]
fn a_new_tab_forgets_its_default_folder_only_once_it_is_proven_gone() {
    gtk_test(
        "ui::window::composition::tabs::tests::a_new_tab_forgets_its_default_folder_only_once_it_is_proven_gone",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};
            use rustix::io::Errno;

            let root = tempfile::tempdir().expect("tab fixture");
            let parked = root.path().join("parked");
            let stalled = root.path().join("stalled");
            let flaky = root.path().join("flaky");
            let unmounted = root.path().join("nas");
            for directory in [&parked, &stalled, &flaky, &unmounted] {
                std::fs::create_dir_all(directory).expect("default folder");
            }
            let (gate, stall) = (
                crate::test_support::Gate::default(),
                crate::test_support::Gate::default(),
            );
            let _scope = FsQuirks::install(
                root.path(),
                Quirks {
                    before: vec![
                        (FsOp::FolderStamp, "parked", 0, gate.racer()),
                        (FsOp::FolderStamp, "stalled", 0, stall.racer()),
                    ],
                    fail: vec![(FsOp::FolderStamp, "flaky", 0, Errno::IO)],
                    ..Quirks::default()
                },
            );
            let home = crate::ui::window::home_directory();
            let app = application();
            let new_tab = |default: &std::path::Path| {
                let (window, tabs) = open_in(&app);
                tabs.preferences
                    .set_default_directory(Some(default.to_path_buf()));
                tabs.new_tab();
                (window, tabs.active_browser().browser())
            };

            let (parked_window, browser) = new_tab(&parked);
            wait_until(|| gate.reached());
            assert_eq!(browser.active_location(), None, "opens once checked");
            gate.release();
            wait_until(|| browser.active_location() == Some(Location::local(&parked)));
            assert_eq!(
                PreferenceManager::shared().default_directory(),
                Some(parked.clone())
            );

            let (stalled_window, browser) = new_tab(&stalled);
            wait_until(|| stall.reached());
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| column.error.is_some())
            });
            assert_eq!(browser.active_location(), Some(Location::local(&stalled)));
            assert_eq!(
                PreferenceManager::shared().default_directory(),
                Some(stalled.clone()),
                "a check that does not answer opens the folder to show why"
            );
            stall.release();

            let mut windows = vec![parked_window, stalled_window];
            for (default, why) in [
                (
                    flaky.clone(),
                    "an I/O error is not proof that the folder is gone",
                ),
                (
                    unmounted.join("Documents"),
                    "a folder missing from an empty mount point is kept",
                ),
                (
                    root.path().join("drive/Photos"),
                    "a folder missing with its drive is kept",
                ),
            ] {
                let (window, browser) = new_tab(&default);
                wait_until(|| browser.active_location() == Some(Location::local(&default)));
                assert_eq!(
                    PreferenceManager::shared().default_directory(),
                    Some(default.clone()),
                    "{why}"
                );
                windows.push(window);
            }

            let (gone_window, browser) = new_tab(&root.path().join("gone"));
            wait_until(|| browser.active_location() == Some(Location::local(&home)));
            assert_eq!(PreferenceManager::shared().default_directory(), None);
            windows.push(gone_window);
            for window in windows {
                window.destroy();
            }
        },
    );
}

#[test]
fn tab_changes_persist_for_the_next_launch() {
    gtk_test(
        "ui::window::composition::tabs::tests::tab_changes_persist_for_the_next_launch",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            let alpha = root.path().join("alpha");
            let beta = root.path().join("beta");
            for directory in [&alpha, &beta] {
                std::fs::create_dir_all(directory).expect("session directory");
            }
            let app = application();
            let (first_window, first) = open_in(&app);
            load(&first.active_browser(), Location::local(&alpha));
            wait_until(|| {
                saved_session().is_some_and(|session| session.tabs == vec![Location::local(&alpha)])
            });
            let (second_window, second) = open_in(&app);
            load(&second.active_browser(), Location::local(&beta));
            wait_until(|| {
                saved_session().is_some_and(|session| session.tabs == vec![Location::local(&beta)])
            });
            let (third_window, third) = open_in(&app);
            assert!(third.try_restore());
            wait_until(|| tab_locations(&third) == vec![Some(Location::local(&beta))]);
            assert_eq!(third.tabs.borrow().len(), 1);
            first_window.destroy();
            second_window.destroy();
            third_window.destroy();
        },
    );
}

#[test]
fn disabled_restore_ignores_the_saved_session() {
    gtk_test(
        "ui::window::composition::tabs::tests::disabled_restore_ignores_the_saved_session",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            std::fs::create_dir_all(root.path().join("kept")).expect("session directory");
            let (window, tabs) = open();
            tabs.preferences.set_restore_tabs(false);
            crate::ui::tabs_session::save(&[Location::local(root.path().join("kept"))], 0);
            crate::storage::writer::settle();
            assert!(!tabs.try_restore());
            assert_eq!(tabs.tabs.borrow().len(), 1);
            assert_eq!(tab_locations(&tabs), vec![None]);
            window.destroy();
        },
    );
}

#[test]
fn explicit_target_windows_do_not_clobber_the_saved_session() {
    gtk_test(
        "ui::window::composition::tabs::tests::explicit_target_windows_do_not_clobber_the_saved_session",
        || {
            let root = tempfile::tempdir().expect("tab fixture");
            let kept = root.path().join("kept");
            let other = root.path().join("other");
            for directory in [&kept, &other] {
                std::fs::create_dir_all(directory).expect("session directory");
            }
            crate::ui::tabs_session::save(&[Location::local(&kept)], 0);
            crate::storage::writer::settle();
            let (window, tabs) = open_persisting(&application(), false);
            load(&tabs.active_browser(), Location::local(&other));
            crate::storage::writer::settle();
            assert_eq!(
                saved_session().map(|session| session.tabs),
                Some(vec![Location::local(&kept)])
            );
            assert_eq!(tabs.tabs.borrow().len(), 1);
            window.destroy();
        },
    );
}

#[test]
fn tab_drop_slots_preserve_the_active_browser_and_saved_order() {
    gtk_test(
        "ui::window::composition::tabs::tests::tab_drop_slots_preserve_the_active_browser_and_saved_order",
        || {
            let root = tempfile::tempdir().expect("tab folders");
            let (window, tabs) = open();
            tabs.preferences.set_restore_tabs(true);
            let locations: Vec<_> = ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|name| {
                    let path = root.path().join(name);
                    std::fs::create_dir(&path).expect("folder");
                    Location::local(path)
                })
                .collect();
            load(&tabs.active_browser(), locations[0].clone());
            for location in &locations[1..] {
                tabs.open_in_new_tab(location.clone());
                wait_until(|| {
                    tabs.active_browser().browser().active_location().as_ref() == Some(location)
                });
            }
            let active = tabs.active.get();
            let browser = tabs.active_browser().browser();
            for (source, insertion, order) in [
                (1, 3, [2, 3, 1]),
                (1, 1, [2, 1, 3]),
                (3, 0, [3, 2, 1]),
                (3, 1, [3, 2, 1]),
                (2, 2, [3, 2, 1]),
                (2, 3, [3, 1, 2]),
                (99, 0, [3, 1, 2]),
                (1, 4, [3, 1, 2]),
            ] {
                tabs.reorder_at(source, insertion);
                assert_eq!(
                    tabs.tabs
                        .borrow()
                        .iter()
                        .map(|tab| tab.id)
                        .collect::<Vec<_>>(),
                    order
                );
                assert_eq!(tabs.active.get(), active);
                assert!(Rc::ptr_eq(&tabs.active_browser().browser(), &browser));
                let expected: Vec<_> = order
                    .iter()
                    .map(|id| locations[*id as usize - 1].clone())
                    .collect();
                wait_until(|| saved_session().is_some_and(|session| session.tabs == expected));
                assert_eq!(
                    saved_session().expect("saved tabs").active,
                    order
                        .iter()
                        .position(|id| *id == active)
                        .expect("active tab")
                );
            }
            window.destroy();
        },
    );
}

#[test]
fn sidebar_tab_actions_preserve_the_original_tab() {
    gtk_test(
        "ui::window::composition::tabs::tests::sidebar_tab_actions_preserve_the_original_tab",
        || {
            let application = application();
            for from_menu in [false, true] {
                let root = tempfile::tempdir().expect("original folder");
                let (window, tabs) = open_in(&application);
                tabs.preferences.set_sidebar_show_home(true);
                let original = tabs.active_browser();
                load(&original, Location::local(root.path()));
                let home = Location::local(glib::home_dir());
                let row = tabs
                    .active_tab()
                    .content
                    .sidebar
                    .state
                    .place_rows
                    .borrow()
                    .iter()
                    .find(|(location, _)| location == &home)
                    .map(|(_, row)| row.clone())
                    .expect("Home row");
                if from_menu {
                    let option = find_widget(row.upcast_ref(), &|widget| {
                        widget
                            .downcast_ref::<gtk::Label>()
                            .is_some_and(|label| label.text() == "Open in New Tab")
                    })
                    .expect("new tab menu label")
                    .ancestor(gtk::Button::static_type())
                    .and_downcast::<gtk::Button>()
                    .expect("menu button");
                    option.emit_clicked();
                } else {
                    let controllers = row.observe_controllers();
                    let middle = (0..controllers.n_items())
                        .filter_map(|index| {
                            controllers.item(index).and_downcast::<gtk::GestureClick>()
                        })
                        .find(|gesture| gesture.button() == 2)
                        .expect("middle click");
                    middle.emit_by_name::<()>("released", &[&1_i32, &0_f64, &0_f64]);
                }
                wait_until(|| {
                    tabs.active_browser().browser().active_location() == Some(home.clone())
                });
                assert_eq!(tabs.tabs.borrow().len(), 2);
                tabs.select_index(0);
                assert!(Rc::ptr_eq(
                    &tabs.active_browser().browser(),
                    &original.browser()
                ));
                assert_eq!(
                    original.browser().active_location(),
                    Some(Location::local(root.path()))
                );
                window.destroy();
            }
        },
    );
}

fn open_target_fixture() -> (
    tempfile::TempDir,
    gtk::ApplicationWindow,
    Rc<TabWindow>,
    Location,
) {
    PreferenceManager::seed_saved_preferences_for_test();
    let root = tempfile::tempdir().expect("open target fixture");
    let destination = root.path().join("destination");
    std::fs::create_dir(&destination).expect("destination directory");
    let (window, tabs) = open();
    load(&tabs.active_browser(), Location::local(root.path()));
    let location = Location::local(&destination);
    (root, window, tabs, location)
}

#[test]
fn open_in_new_tab_appends_activates_and_navigates_the_target() {
    gtk_test(
        "ui::window::composition::tabs::tests::open_in_new_tab_appends_activates_and_navigates_the_target",
        || {
            let (_root, window, tabs, location) = open_target_fixture();
            let first = tabs.active.get();
            tabs.open_in_new_tab(location.clone());
            let opened = tabs.active.get();
            assert_ne!(opened, first, "the new tab becomes active");
            assert_eq!(
                tabs.tabs
                    .borrow()
                    .iter()
                    .map(|tab| tab.id)
                    .collect::<Vec<_>>(),
                vec![first, opened],
                "the new tab is appended last"
            );
            let view = tabs.active_browser();
            wait_until(|| view.browser().active_location().is_some());
            assert_eq!(view.browser().active_location(), Some(location));
            window.destroy();
        },
    );
}

#[test]
fn open_in_new_tab_reuses_the_unavailable_overlay_for_a_missing_target() {
    gtk_test(
        "ui::window::composition::tabs::tests::open_in_new_tab_reuses_the_unavailable_overlay_for_a_missing_target",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("missing target fixture");
            let missing = root.path().join("missing");
            let (window, tabs) = open();
            load(&tabs.active_browser(), Location::local(root.path()));
            tabs.open_in_new_tab(Location::local(&missing));
            let view = tabs.active_browser();
            wait_until(|| button_with_label(view.overlay().upcast_ref(), "Retry").is_some());
            assert!(
                button_with_label(view.overlay().upcast_ref(), "Retry").is_some(),
                "a missing target shows the retry overlay"
            );
            window.destroy();
        },
    );
}

#[test]
fn open_tab_at_action_reaches_the_new_tab_helper() {
    gtk_test(
        "ui::window::composition::tabs::tests::open_tab_at_action_reaches_the_new_tab_helper",
        || {
            let (_root, window, tabs, location) = open_target_fixture();
            let first = tabs.active.get();
            let value = location_action_value(&location);
            gio::prelude::ActionGroupExt::activate_action(&window, "open-tab-at", Some(&value));
            assert_ne!(tabs.active.get(), first);
            assert_eq!(tabs.tabs.borrow().len(), 2);
            window.destroy();
        },
    );
}

#[test]
fn open_in_new_window_presents_a_window_at_the_target() {
    gtk_test(
        "ui::window::composition::tabs::tests::open_in_new_window_presents_a_window_at_the_target",
        || {
            let (_root, window, tabs, location) = open_target_fixture();
            let before = gtk::Window::list_toplevels().len();
            let view = tabs
                .open_in_new_window(location.clone())
                .expect("new window browser");
            assert_eq!(
                gtk::Window::list_toplevels().len(),
                before + 1,
                "a new window is presented"
            );
            wait_until(|| view.browser().active_location().is_some());
            assert_eq!(view.browser().active_location(), Some(location));
            if let Some(opened) = view.overlay().root().and_downcast::<gtk::Window>() {
                opened.destroy();
            }
            window.destroy();
        },
    );
}
