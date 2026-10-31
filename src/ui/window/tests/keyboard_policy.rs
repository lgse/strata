// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{
    gdk::{Key, ModifierType},
    prelude::*,
};

use super::*;
use crate::ui::{keys::KeyPress, preferences::PreferenceManager};

const NONE: ModifierType = ModifierType::empty();
const CONTROL: ModifierType = ModifierType::CONTROL_MASK;
const SHIFT: ModifierType = ModifierType::SHIFT_MASK;
const ALT: ModifierType = ModifierType::ALT_MASK;

fn press(key: Key, modifiers: ModifierType) -> KeyPress {
    KeyPress::from_signal(key, 0, modifiers)
}

#[test]
fn plain_single_pane_arrows_move_focus_not_directories() {
    let action = |mode, key, modifiers, at_left_edge, sidebar_visible| {
        single_pane_arrow_action(mode, &press(key, modifiers), at_left_edge, sidebar_visible)
    };
    assert_eq!(
        action(BrowserMode::Icons, Key::Left, NONE, false, true),
        Some(SinglePaneArrow::Native)
    );
    for mode in [BrowserMode::Icons, BrowserMode::List] {
        assert_eq!(
            action(mode, Key::Left, NONE, true, true),
            Some(SinglePaneArrow::Sidebar)
        );
        for key in [Key::Up, Key::Down] {
            assert_eq!(
                action(mode, key, NONE, true, true),
                Some(SinglePaneArrow::Native)
            );
        }
        for key in [Key::Left, Key::Right, Key::Up] {
            assert_eq!(action(mode, key, ALT, true, true), None);
        }
        assert_eq!(action(mode, Key::Return, NONE, true, true), None);
    }
    assert_eq!(
        action(BrowserMode::List, Key::Right, NONE, true, true),
        Some(SinglePaneArrow::Stay)
    );
    assert_eq!(
        action(BrowserMode::List, Key::Left, NONE, true, false),
        Some(SinglePaneArrow::Stay)
    );
    assert_eq!(
        action(BrowserMode::Icons, Key::Left, NONE, true, false),
        Some(SinglePaneArrow::Native)
    );
    assert_eq!(
        action(BrowserMode::Columns, Key::Left, NONE, true, true),
        None
    );
    for modifier in [SHIFT, CONTROL] {
        assert_eq!(
            action(BrowserMode::Icons, Key::Left, modifier, true, true),
            Some(SinglePaneArrow::Native)
        );
    }
}

#[test]
fn sidebar_arrows_and_vim_keys_share_focus_directions() {
    for (arrow, vim) in [
        (Key::Left, Key::h),
        (Key::Right, Key::l),
        (Key::Up, Key::k),
        (Key::Down, Key::j),
    ] {
        assert_eq!(
            sidebar_focus_direction(&press(arrow, NONE), NONE),
            vim_focus_direction(&press(vim, NONE))
        );
    }
    assert_eq!(
        sidebar_focus_direction(&press(Key::Return, NONE), NONE),
        None
    );
    assert_eq!(sidebar_focus_direction(&press(Key::Up, SHIFT), NONE), None);
    assert_eq!(
        sidebar_focus_direction(&press(Key::Up, SHIFT), SHIFT),
        Some(gtk::DirectionType::Up)
    );
}

#[test]
fn navigation_keys_claim_keyboard_ownership_but_commands_do_not() {
    for key in [
        Key::Up,
        Key::Down,
        Key::h,
        Key::j,
        Key::k,
        Key::l,
        Key::Tab,
        Key::ISO_Left_Tab,
        Key::Page_Down,
        Key::Home,
        Key::End,
        Key::KP_Home,
        Key::KP_End,
        Key::Return,
    ] {
        assert!(is_browser_navigation_key(&press(key, NONE)), "{key:?}");
    }
    assert!(is_browser_navigation_key(&press(Key::Down, SHIFT)));
    assert!(is_browser_navigation_key(&press(Key::Left, ALT)));
    for key in [Key::v, Key::c, Key::x, Key::z, Key::Control_L, Key::Delete] {
        assert!(!is_browser_navigation_key(&press(key, CONTROL)), "{key:?}");
    }
}

#[test]
fn mouse_history_buttons_map_to_navigation_actions() {
    assert_eq!(mouse_history_action(8), Some(MouseHistoryAction::Back));
    assert_eq!(mouse_history_action(9), Some(MouseHistoryAction::Forward));
    for button in [1, 2, 3, 4, 5, 6, 7, 10] {
        assert_eq!(mouse_history_action(button), None);
    }
}

#[test]
fn type_to_search_accepts_printable_keys_without_command_modifiers() {
    assert_eq!(
        type_to_search_query(&press(Key::a, NONE)),
        Some(TypeToSearchQuery::Character('a'))
    );
    assert_eq!(
        type_to_search_query(&press(Key::A, SHIFT)),
        Some(TypeToSearchQuery::Character('A'))
    );
    assert_eq!(
        type_to_search_query(&press(Key::period, NONE)),
        Some(TypeToSearchQuery::Character('.'))
    );
}

#[test]
fn type_to_search_uses_slash_to_open_an_empty_filter() {
    assert_eq!(
        type_to_search_query(&press(Key::slash, NONE)),
        Some(TypeToSearchQuery::Empty)
    );
    // Icelandic types þ on the US `/` key.
    assert_eq!(
        type_to_search_query(&KeyPress::from_signal(Key::thorn, 61, NONE)),
        Some(TypeToSearchQuery::Character('þ'))
    );
}

#[test]
fn type_to_search_leaves_space_for_quick_preview() {
    for modifiers in [NONE, SHIFT] {
        assert_eq!(type_to_search_query(&press(Key::space, modifiers)), None);
    }
}

#[test]
fn type_to_search_ignores_shortcuts_and_non_printable_keys() {
    assert_eq!(type_to_search_query(&press(Key::k, CONTROL)), None);
    assert_eq!(type_to_search_query(&press(Key::F5, NONE)), None);
}

#[test]
fn vim_focus_keys_map_to_gtk_directions() {
    for (key, direction) in [
        (Key::h, gtk::DirectionType::Left),
        (Key::j, gtk::DirectionType::Down),
        (Key::k, gtk::DirectionType::Up),
        (Key::l, gtk::DirectionType::Right),
    ] {
        assert_eq!(vim_focus_direction(&press(key, NONE)), Some(direction));
    }
    assert_eq!(vim_focus_direction(&press(Key::Down, NONE)), None);
}

#[test]
fn control_digits_select_each_browser_presentation() {
    for (key, mode) in [
        (Key::_1, BrowserMode::Columns),
        (Key::_2, BrowserMode::Icons),
        (Key::_3, BrowserMode::List),
        (Key::KP_3, BrowserMode::List),
    ] {
        assert_eq!(browser_mode_for_digit(&press(key, CONTROL)), Some(mode));
    }
    assert_eq!(browser_mode_for_digit(&press(Key::_4, CONTROL)), None);
    assert_eq!(browser_mode_for_digit(&press(Key::a, CONTROL)), None);
    assert_eq!(browser_mode_for_digit(&press(Key::_1, NONE)), None);
}

const WINDOW_SHORTCUTS: [(Key, ModifierType, &str); 5] = [
    (Key::k, CONTROL, "search"),
    (Key::K, CONTROL.union(SHIFT), "jump-folder"),
    (Key::t, CONTROL.union(ALT), "open-terminal"),
    (Key::F5, NONE, "refresh"),
    (Key::backslash, CONTROL, "toggle-arrow-scope"),
];

#[test]
fn window_actions_run_once_the_dispatcher_yields_and_claim_no_other_key() {
    gtk_test(
        "ui::window::tests::keyboard_policy::window_actions_run_once_the_dispatcher_yields_and_claim_no_other_key",
        || {
            PreferenceManager::shared().set_tenxer_mode(false);
            let (window, content) = policy_window();
            let runs = record_window_actions(&window);
            focus_location(&content);
            for (key, modifiers, action) in WINDOW_SHORTCUTS {
                assert!(dispatch(&window, key, modifiers), "{action}");
                assert_eq!(runs.take(), [action]);
            }
            // F2 and Ctrl+R rename; the window's F5 is exact, as GTK's accelerator was.
            for (key, modifiers) in [
                (Key::F2, NONE),
                (Key::r, CONTROL),
                (Key::r, NONE),
                (Key::F5, SHIFT),
                (Key::k, CONTROL | ALT),
                (Key::t, NONE),
                (Key::t, CONTROL),
                (Key::t, CONTROL | SHIFT),
                (Key::t, CONTROL | ALT | SHIFT),
                (Key::F4, CONTROL),
            ] {
                dispatch(&window, key, modifiers);
                assert!(runs.take().is_empty(), "{key:?} ran a window action");
            }
            window.destroy();
        },
    );
}

#[test]
fn the_open_reference_yields_input_method_keys_and_runs_no_window_action() {
    gtk_test(
        "ui::window::tests::keyboard_policy::the_open_reference_yields_input_method_keys_and_runs_no_window_action",
        || {
            PreferenceManager::shared().set_tenxer_mode(false);
            let (window, _content) = policy_window();
            let runs = record_window_actions(&window);
            assert!(dispatch(&window, Key::F1, NONE));
            settle_policy();
            assert!(reference_search_has_focus(&window));
            for modifiers in [CONTROL, ModifierType::SUPER_MASK] {
                assert!(
                    !dispatch(&window, Key::space, modifiers),
                    "{modifiers:?}+Space must reach the search's input method"
                );
            }
            for (key, modifiers, action) in WINDOW_SHORTCUTS {
                dispatch(&window, key, modifiers);
                assert!(
                    runs.take().is_empty(),
                    "{action} ran while the reference was open"
                );
            }
            assert!(reference_search_has_focus(&window));
            window.destroy();
        },
    );
}

fn reference_search_has_focus(window: &gtk::ApplicationWindow) -> bool {
    gtk::prelude::RootExt::focus(window).is_some_and(|focus| {
        focus
            .ancestor(gtk::Entry::static_type())
            .is_some_and(|search| search.has_css_class("shortcut-reference-search"))
    })
}

#[test]
fn tenxer_window_actions_follow_the_saved_mode_across_windows() {
    gtk_test(
        "ui::window::tests::keyboard_policy::tenxer_window_actions_follow_the_saved_mode_across_windows",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            let (first, first_content) = policy_window();
            let first_runs = record_window_actions(&first);
            focus_location(&first_content);
            assert_window_actions(&first, &first_runs, false);
            preferences.set_tenxer_mode(true);
            assert_window_actions(&first, &first_runs, true);
            let (second, second_content) = policy_window();
            let second_runs = record_window_actions(&second);
            focus_location(&second_content);
            assert_window_actions(&second, &second_runs, true);
            first.destroy();
            settle_policy();
            assert_window_actions(&second, &second_runs, true);
            preferences.set_tenxer_mode(false);
            assert_window_actions(&second, &second_runs, false);
            second.destroy();
        },
    );
}

fn policy_window() -> (
    gtk::ApplicationWindow,
    super::super::composition::WindowContent,
) {
    let preferences = PreferenceManager::shared();
    let application = gtk::gio::Application::default()
        .and_downcast::<gtk::Application>()
        .unwrap_or_else(|| {
            let application =
                gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("test application registration");
            application
        });
    let window = gtk::ApplicationWindow::builder()
        .application(&application)
        .default_width(800)
        .default_height(600)
        .build();
    let content = super::super::composition::WindowContent::new(&window, &preferences);
    content.bind(&window, &preferences);
    window.present();
    (window, content)
}

fn record_window_actions(window: &gtk::ApplicationWindow) -> Rc<RefCell<Vec<&'static str>>> {
    let runs = Rc::new(RefCell::new(Vec::new()));
    for (_, _, name) in WINDOW_SHORTCUTS {
        let action = gtk::gio::SimpleAction::new(name, None);
        let runs = runs.clone();
        action.connect_activate(move |_, _| runs.borrow_mut().push(name));
        window.add_action(&action);
    }
    runs
}

/// The location field yields every key the dispatcher does not own.
fn focus_location(content: &super::super::composition::WindowContent) {
    content.browser.begin_location_edit();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !content.browser.location_has_focus() {
        assert!(std::time::Instant::now() < deadline, "location focus");
        gtk::glib::MainContext::default().iteration(false);
    }
}

fn dispatch(window: &gtk::ApplicationWindow, key: Key, modifiers: ModifierType) -> bool {
    let controllers = window.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("key dispatcher")
        .emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
}

fn assert_window_actions(
    window: &gtk::ApplicationWindow,
    runs: &RefCell<Vec<&'static str>>,
    tenxer: bool,
) {
    for (key, modifiers, action) in WINDOW_SHORTCUTS {
        let bound = !tenxer || matches!(action, "search" | "refresh");
        assert_eq!(
            dispatch(window, key, modifiers),
            bound,
            "{action}, 10xer={tenxer}"
        );
        let expected: &[&str] = if bound { &[action] } else { &[] };
        assert_eq!(runs.take(), expected, "{action}, 10xer={tenxer}");
    }
}

fn settle_policy() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}
