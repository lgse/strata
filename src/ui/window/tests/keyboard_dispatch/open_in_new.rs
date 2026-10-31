// SPDX-License-Identifier: MIT

use super::*;

const MODES: [BrowserMode; 3] = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons];

/// Records the window actions a `TabWindow` would otherwise handle.
fn record_open_actions(window: &gtk::ApplicationWindow) -> Rc<RefCell<Vec<(String, String)>>> {
    use gtk::gio::prelude::ActionMapExt;

    let recorded = Rc::new(RefCell::new(Vec::new()));
    for name in ["open-tab-at", "open-window-at"] {
        let action = gtk::gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let recorded = recorded.clone();
        action.connect_activate(move |_, parameter| {
            let uri = parameter
                .and_then(|value| value.get::<String>())
                .unwrap_or_default();
            recorded.borrow_mut().push((name.to_string(), uri));
        });
        window.add_action(&action);
    }
    recorded
}

fn open_fixture(fixture: &KeyboardFixture) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("fixture root");
    std::fs::create_dir(root.path().join("folder")).expect("fixture folder");
    fixture
        .view
        .browser()
        .navigate(Location::local(root.path()));
    wait_loaded(&fixture.view.browser(), 0);
    root
}

fn mode_row_named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let is_row = widget.has_css_class("file-row")
        || widget.has_css_class("list-row")
        || widget.has_css_class("icons-card");
    if is_row && rendered_name(widget, name) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = mode_row_named(&widget, name) {
            return Some(found);
        }
    }
    None
}

fn middle_click_row(widget: &gtk::Widget, name: &str, count: i32) -> bool {
    let Some(row) = mode_row_named(widget, name) else {
        return false;
    };
    let controllers = row.observe_controllers();
    let Some(click) = (0..controllers.n_items()).find_map(|index| {
        controllers
            .item(index)
            .and_then(|controller| controller.downcast::<gtk::GestureClick>().ok())
            .filter(|click| click.button() == gtk::gdk::BUTTON_MIDDLE)
    }) else {
        return false;
    };
    click.emit_by_name::<()>("pressed", &[&count, &8.0f64, &8.0f64]);
    click.emit_by_name::<()>("released", &[&count, &8.0f64, &8.0f64]);
    true
}

fn expected_uri(path: &std::path::Path) -> String {
    crate::adapters::gio_file_for_location(&Location::local(path))
        .uri()
        .to_string()
}

#[test]
fn middle_click_opens_a_directory_in_a_new_tab_in_every_view_mode() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::open_in_new::middle_click_opens_a_directory_in_a_new_tab_in_every_view_mode",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let recorded = record_open_actions(&fixture.window);
            let root = open_fixture(&fixture);
            let folder = root.path().join("folder");

            for mode in MODES {
                fixture.view.set_view_mode(mode);
                wait_until(|| fixture.view.view_mode() == mode);
                move_to_named(&fixture, &browser, "folder");
                wait_until(|| mode_row_named(&fixture.view.widget(), "folder").is_some());
                recorded.borrow_mut().clear();

                assert!(
                    middle_click_row(&fixture.view.widget(), "folder", 1),
                    "{mode:?} folder row has a middle-click gesture"
                );
                pump(50);
                let opened = recorded.borrow().clone();
                assert_eq!(opened.len(), 1, "{mode:?} {opened:?}");
                assert_eq!(opened[0].0, "open-tab-at", "{mode:?}");
                assert_eq!(opened[0].1, expected_uri(&folder), "{mode:?}");

                assert!(middle_click_row(&fixture.view.widget(), "folder", 2));
                pump(50);
                assert_eq!(
                    recorded.borrow().len(),
                    1,
                    "{mode:?} a double middle-click opens once"
                );
            }
        },
    );
}

#[test]
fn middle_click_reveals_a_file_and_never_starts_autoscroll_over_a_row() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::open_in_new::middle_click_reveals_a_file_and_never_starts_autoscroll_over_a_row",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let recorded = record_open_actions(&fixture.window);

            for mode in MODES {
                fixture.view.set_view_mode(mode);
                wait_until(|| fixture.view.view_mode() == mode);
                move_to_named(&fixture, &browser, "a.txt");
                wait_until(|| mode_row_named(&fixture.view.widget(), "a.txt").is_some());
                recorded.borrow_mut().clear();

                assert!(middle_click_row(&fixture.view.widget(), "a.txt", 1));
                pump(50);
                assert!(
                    recorded.borrow().is_empty(),
                    "{mode:?} a file row must not open a tab or window"
                );
                assert!(!crate::ui::scrolling::autoscroll_is_running(), "{mode:?}");
                assert!(
                    browser
                        .selected_entries()
                        .iter()
                        .any(|entry| entry.display_name == "a.txt"),
                    "{mode:?} the file is revealed in its parent"
                );
            }
        },
    );
}

#[test]
fn ctrl_and_shift_enter_open_a_directory_in_a_new_tab_or_window() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::open_in_new::ctrl_and_shift_enter_open_a_directory_in_a_new_tab_or_window",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let recorded = record_open_actions(&fixture.window);
            let root = open_fixture(&fixture);
            let folder = root.path().join("folder");
            move_to_named(&fixture, &browser, "folder");

            assert!(fixture.press(Key::Return, ModifierType::CONTROL_MASK));
            pump(50);
            {
                let opened = recorded.borrow();
                assert_eq!(opened.len(), 1, "{opened:?}");
                assert_eq!(opened[0].0, "open-tab-at");
                assert_eq!(opened[0].1, expected_uri(&folder));
            }

            recorded.borrow_mut().clear();
            assert!(fixture.press(Key::Return, ModifierType::SHIFT_MASK));
            pump(50);
            {
                let opened = recorded.borrow();
                assert_eq!(opened.len(), 1, "{opened:?}");
                assert_eq!(opened[0].0, "open-window-at");
                assert_eq!(opened[0].1, expected_uri(&folder));
            }
        },
    );
}

#[test]
fn ctrl_enter_on_a_focused_file_opens_with_the_default_app() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::open_in_new::ctrl_enter_on_a_focused_file_opens_with_the_default_app",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let recorded = record_open_actions(&fixture.window);
            move_to_named(&fixture, &browser, "a.txt");
            assert!(fixture.view.item_view_has_focus());

            assert!(fixture.press(Key::Return, ModifierType::CONTROL_MASK));
            pump(50);
            assert!(
                recorded.borrow().is_empty(),
                "Ctrl+Enter on a file must not open a new tab"
            );
        },
    );
}

#[test]
fn shift_enter_on_a_focused_file_opens_with_the_default_app() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::open_in_new::shift_enter_on_a_focused_file_opens_with_the_default_app",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let recorded = record_open_actions(&fixture.window);
            move_to_named(&fixture, &browser, "a.txt");
            assert!(fixture.view.item_view_has_focus());

            assert!(fixture.press(Key::Return, ModifierType::SHIFT_MASK));
            pump(50);
            assert!(
                recorded.borrow().is_empty(),
                "Shift+Enter on a file must not open a new window"
            );
        },
    );
}

#[test]
fn tenxer_ctrl_and_shift_enter_open_a_directory_in_a_new_tab_or_window() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::open_in_new::tenxer_ctrl_and_shift_enter_open_a_directory_in_a_new_tab_or_window",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let recorded = record_open_actions(&fixture.window);
            let root = open_fixture(&fixture);
            let folder = root.path().join("folder");
            let preferences = PreferenceManager::shared();
            fixture.shortcuts.bind_preferences(&preferences);
            preferences.set_tenxer_mode(true);
            pump(50);
            move_to_named(&fixture, &browser, "folder");

            assert!(fixture.press(Key::Return, ModifierType::CONTROL_MASK));
            pump(50);
            {
                let opened = recorded.borrow();
                assert_eq!(opened.len(), 1, "{opened:?}");
                assert_eq!(opened[0].0, "open-tab-at");
                assert_eq!(opened[0].1, expected_uri(&folder));
            }

            recorded.borrow_mut().clear();
            assert!(fixture.press(Key::Return, ModifierType::SHIFT_MASK));
            pump(50);
            {
                let opened = recorded.borrow();
                assert_eq!(opened.len(), 1, "{opened:?}");
                assert_eq!(opened[0].0, "open-window-at");
            }
        },
    );
}
