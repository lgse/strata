// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::*;
use crate::ui::preferences::PreferenceManager;

#[test]
fn plain_single_pane_arrows_move_focus_not_directories() {
    use super::{BrowserMode, SinglePaneArrow, single_pane_arrow_action};
    use gtk::gdk::{Key, ModifierType};
    let plain = ModifierType::empty();
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Icons, Key::Left, plain, false, true),
        Some(SinglePaneArrow::Native)
    );
    for mode in [BrowserMode::Icons, BrowserMode::List] {
        assert_eq!(
            single_pane_arrow_action(mode, Key::Left, plain, true, true),
            Some(SinglePaneArrow::Sidebar)
        );
        for key in [Key::Up, Key::Down] {
            assert_eq!(
                single_pane_arrow_action(mode, key, plain, true, true),
                Some(SinglePaneArrow::Native)
            );
        }
        for key in [Key::Left, Key::Right, Key::Up] {
            assert_eq!(
                single_pane_arrow_action(mode, key, ModifierType::ALT_MASK, true, true),
                None
            );
        }
        assert_eq!(
            single_pane_arrow_action(mode, Key::Return, plain, true, true),
            None
        );
    }
    assert_eq!(
        single_pane_arrow_action(BrowserMode::List, Key::Right, plain, true, true),
        Some(SinglePaneArrow::Stay)
    );
    assert_eq!(
        single_pane_arrow_action(BrowserMode::List, Key::Left, plain, true, false),
        Some(SinglePaneArrow::Stay)
    );
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Icons, Key::Left, plain, true, false),
        Some(SinglePaneArrow::Native)
    );
    assert_eq!(
        single_pane_arrow_action(BrowserMode::Columns, Key::Left, plain, true, true),
        None
    );
    for modifier in [ModifierType::SHIFT_MASK, ModifierType::CONTROL_MASK] {
        assert_eq!(
            single_pane_arrow_action(BrowserMode::Icons, Key::Left, modifier, true, true),
            Some(SinglePaneArrow::Native)
        );
    }
}

#[test]
fn sidebar_arrows_and_vim_keys_share_focus_directions() {
    use gtk::gdk::Key;
    for (arrow, vim) in [
        (Key::Left, Key::h),
        (Key::Right, Key::l),
        (Key::Up, Key::k),
        (Key::Down, Key::j),
    ] {
        assert_eq!(
            super::sidebar_focus_direction(arrow),
            vim_focus_direction(vim)
        );
    }
    assert_eq!(super::sidebar_focus_direction(Key::Return), None);
}

#[test]
fn navigation_keys_claim_keyboard_ownership_but_commands_do_not() {
    use gtk::gdk::{Key, ModifierType};
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
        Key::Return,
    ] {
        assert!(super::is_browser_navigation_key(key, ModifierType::empty()));
    }
    assert!(super::is_browser_navigation_key(
        Key::Down,
        ModifierType::SHIFT_MASK
    ));
    assert!(super::is_browser_navigation_key(
        Key::Left,
        ModifierType::ALT_MASK
    ));
    for key in [Key::v, Key::c, Key::x, Key::z, Key::Control_L, Key::Delete] {
        assert!(!super::is_browser_navigation_key(
            key,
            ModifierType::CONTROL_MASK
        ));
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
fn open_terminal_shortcut_requires_only_control() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_open_terminal_shortcut(gtk::gdk::Key::t, control));
    assert!(is_open_terminal_shortcut(gtk::gdk::Key::T, control));
    assert!(!is_open_terminal_shortcut(
        gtk::gdk::Key::t,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_open_terminal_shortcut(
        gtk::gdk::Key::t,
        control | shift
    ));
    assert!(!is_open_terminal_shortcut(gtk::gdk::Key::t, control | alt));
    assert!(!is_open_terminal_shortcut(gtk::gdk::Key::F4, control));
}

#[test]
fn undo_shortcut_requires_control_without_shift_or_alt() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_undo_shortcut(gtk::gdk::Key::z, control));
    assert!(is_undo_shortcut(gtk::gdk::Key::Z, control));
    assert!(!is_undo_shortcut(
        gtk::gdk::Key::z,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_undo_shortcut(gtk::gdk::Key::z, control | shift));
    assert!(!is_undo_shortcut(gtk::gdk::Key::z, control | alt));
}

#[test]
fn redo_shortcut_accepts_control_shift_z_and_control_y() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_redo_shortcut(gtk::gdk::Key::z, control | shift));
    assert!(is_redo_shortcut(gtk::gdk::Key::Z, control | shift));
    assert!(is_redo_shortcut(gtk::gdk::Key::y, control));
    assert!(is_redo_shortcut(gtk::gdk::Key::Y, control));
    assert!(!is_redo_shortcut(gtk::gdk::Key::z, control));
    assert!(!is_redo_shortcut(gtk::gdk::Key::z, control | shift | alt));
    assert!(!is_redo_shortcut(gtk::gdk::Key::y, control | shift));
    assert!(!is_redo_shortcut(gtk::gdk::Key::y, control | alt));
    assert!(!is_redo_shortcut(gtk::gdk::Key::z, shift));
    assert!(!is_redo_shortcut(
        gtk::gdk::Key::z,
        gtk::gdk::ModifierType::empty()
    ));
}

#[test]
fn page_keys_map_to_a_scroll_direction() {
    assert_eq!(page_direction(gtk::gdk::Key::Page_Up), Some(-1));
    assert_eq!(page_direction(gtk::gdk::Key::KP_Page_Up), Some(-1));
    assert_eq!(page_direction(gtk::gdk::Key::Page_Down), Some(1));
    assert_eq!(page_direction(gtk::gdk::Key::KP_Page_Down), Some(1));
    assert_eq!(page_direction(gtk::gdk::Key::Home), None);
}

#[test]
fn jump_shortcut_requires_control_without_other_command_modifiers() {
    use gtk::gdk::{Key, ModifierType};
    let control = ModifierType::CONTROL_MASK;

    assert_eq!(jump_direction(Key::Up, control), Some(-1));
    assert_eq!(jump_direction(Key::Down, control), Some(1));
    assert_eq!(jump_direction(Key::Left, control), None);
    assert_eq!(jump_direction(Key::Up, ModifierType::empty()), None);
    for modifier in [
        ModifierType::SHIFT_MASK,
        ModifierType::ALT_MASK,
        ModifierType::SUPER_MASK,
    ] {
        assert_eq!(jump_direction(Key::Up, control | modifier), None);
    }
}

#[test]
fn toggle_hidden_shortcut_accepts_h_or_period_with_only_control() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_toggle_hidden_shortcut(gtk::gdk::Key::h, control));
    assert!(is_toggle_hidden_shortcut(gtk::gdk::Key::H, control));
    assert!(is_toggle_hidden_shortcut(gtk::gdk::Key::period, control));
    assert!(!is_toggle_hidden_shortcut(
        gtk::gdk::Key::h,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_toggle_hidden_shortcut(
        gtk::gdk::Key::h,
        control | shift
    ));
    assert!(!is_toggle_hidden_shortcut(gtk::gdk::Key::h, control | alt));
    assert!(!is_toggle_hidden_shortcut(
        gtk::gdk::Key::period,
        control | shift
    ));
}

#[test]
fn sidebar_focus_shortcut_requires_control_and_shift() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;

    assert!(is_sidebar_focus_shortcut(gtk::gdk::Key::b, control | shift));
    assert!(is_sidebar_focus_shortcut(gtk::gdk::Key::B, control | shift));
    assert!(!is_sidebar_focus_shortcut(gtk::gdk::Key::b, control));
}

#[test]
fn context_menu_shortcut_accepts_menu_key_alone_and_shift_f10() {
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_context_menu_shortcut(
        gtk::gdk::Key::Menu,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(is_context_menu_shortcut(gtk::gdk::Key::F10, shift));
    assert!(!is_context_menu_shortcut(gtk::gdk::Key::Menu, shift));
    assert!(!is_context_menu_shortcut(gtk::gdk::Key::Menu, control));
    assert!(!is_context_menu_shortcut(
        gtk::gdk::Key::F10,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_context_menu_shortcut(
        gtk::gdk::Key::F10,
        shift | control
    ));
    assert!(!is_context_menu_shortcut(gtk::gdk::Key::F10, shift | alt));
    for modifier in [
        gtk::gdk::ModifierType::SUPER_MASK,
        gtk::gdk::ModifierType::HYPER_MASK,
        gtk::gdk::ModifierType::META_MASK,
    ] {
        assert!(!is_context_menu_shortcut(
            gtk::gdk::Key::F10,
            shift | modifier
        ));
        assert!(!is_context_menu_shortcut(gtk::gdk::Key::Menu, modifier));
    }
    assert!(is_context_menu_shortcut(
        gtk::gdk::Key::Menu,
        gtk::gdk::ModifierType::LOCK_MASK
    ));
}

#[test]
fn type_to_search_accepts_printable_keys_without_command_modifiers() {
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::a, gtk::gdk::ModifierType::empty()),
        Some(TypeToSearchQuery::Character('a'))
    );
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::A, gtk::gdk::ModifierType::SHIFT_MASK),
        Some(TypeToSearchQuery::Character('A'))
    );
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::period, gtk::gdk::ModifierType::empty()),
        Some(TypeToSearchQuery::Character('.'))
    );
}

#[test]
fn type_to_search_uses_slash_to_open_an_empty_filter() {
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::slash, gtk::gdk::ModifierType::empty()),
        Some(TypeToSearchQuery::Empty)
    );
}

#[test]
fn type_to_search_leaves_space_for_quick_preview() {
    for modifiers in [
        gtk::gdk::ModifierType::empty(),
        gtk::gdk::ModifierType::SHIFT_MASK,
    ] {
        assert_eq!(type_to_search_query(gtk::gdk::Key::space, modifiers), None);
    }
}

#[test]
fn type_to_search_ignores_shortcuts_and_non_printable_keys() {
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::k, gtk::gdk::ModifierType::CONTROL_MASK),
        None
    );
    assert_eq!(
        type_to_search_query(gtk::gdk::Key::F5, gtk::gdk::ModifierType::empty()),
        None
    );
}

#[test]
fn vim_focus_keys_map_to_gtk_directions() {
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::h),
        Some(gtk::DirectionType::Left)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::j),
        Some(gtk::DirectionType::Down)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::k),
        Some(gtk::DirectionType::Up)
    );
    assert_eq!(
        vim_focus_direction(gtk::gdk::Key::l),
        Some(gtk::DirectionType::Right)
    );
    assert_eq!(vim_focus_direction(gtk::gdk::Key::Down), None);
}

#[test]
fn control_digits_select_each_browser_presentation() {
    use super::BrowserMode;

    assert_eq!(
        browser_mode_for_digit(gtk::gdk::Key::_1),
        Some(BrowserMode::Columns)
    );
    assert_eq!(
        browser_mode_for_digit(gtk::gdk::Key::_2),
        Some(BrowserMode::Icons)
    );
    assert_eq!(
        browser_mode_for_digit(gtk::gdk::Key::_3),
        Some(BrowserMode::List)
    );
    assert_eq!(
        browser_mode_for_digit(gtk::gdk::Key::KP_3),
        Some(BrowserMode::List)
    );
    assert_eq!(browser_mode_for_digit(gtk::gdk::Key::_4), None);
    assert_eq!(browser_mode_for_digit(gtk::gdk::Key::a), None);
}

#[test]
fn rename_shortcut_accepts_f2_and_control_r() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    assert!(is_rename_shortcut(
        gtk::gdk::Key::F2,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(is_rename_shortcut(gtk::gdk::Key::r, control));
    assert!(is_rename_shortcut(gtk::gdk::Key::R, control));
}

#[test]
fn rename_shortcut_ignores_extra_modifiers_and_other_keys() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    assert!(!is_rename_shortcut(
        gtk::gdk::Key::r,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_rename_shortcut(
        gtk::gdk::Key::r,
        control | gtk::gdk::ModifierType::SHIFT_MASK
    ));
    assert!(!is_rename_shortcut(
        gtk::gdk::Key::r,
        control | gtk::gdk::ModifierType::ALT_MASK
    ));
    assert!(!is_rename_shortcut(gtk::gdk::Key::F2, control));
    assert!(!is_rename_shortcut(gtk::gdk::Key::F5, control));
}

#[test]
fn refresh_shortcut_keeps_f5_and_releases_control_r() {
    assert!(is_refresh_shortcut(gtk::gdk::Key::F5));
    assert!(!is_refresh_shortcut(gtk::gdk::Key::r));
    assert!(!is_rename_shortcut(
        gtk::gdk::Key::F5,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(is_rename_shortcut(
        gtk::gdk::Key::r,
        gtk::gdk::ModifierType::CONTROL_MASK
    ));
}

#[test]
fn default_accels_never_bind_one_chord_twice() {
    gtk_test(
        "ui::window::tests::keyboard_policy::default_accels_never_bind_one_chord_twice",
        || {
            let mut seen = std::collections::HashMap::new();
            for (action, accels) in DEFAULT_ACCELS {
                for accel in *accels {
                    let chord = gtk::accelerator_parse(*accel).expect("valid default accelerator");
                    assert!(
                        !is_rename_shortcut(chord.0, chord.1),
                        "{action} must not claim a rename shortcut"
                    );
                    assert!(
                        seen.insert(chord, *action).is_none(),
                        "{accel} is bound to more than one action"
                    );
                }
            }
            let refresh = DEFAULT_ACCELS
                .iter()
                .find(|(action, _)| *action == "win.refresh")
                .expect("refresh accels")
                .1;
            assert_eq!(refresh, &["F5"]);
        },
    );
}

#[test]
fn omastrata_accelerators_follow_the_saved_mode_across_windows() {
    gtk_test(
        "ui::window::tests::keyboard_policy::omastrata_accelerators_follow_the_saved_mode_across_windows",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_omastrata_mode(false);
            let first = policy_window();
            assert_accelerators(&first, false);
            preferences.set_omastrata_mode(true);
            assert_accelerators(&first, true);
            let enabled = accel_snapshot(&first);
            let second = policy_window();
            assert_eq!(accel_snapshot(&second), enabled);
            first.destroy();
            settle_policy();
            assert_eq!(accel_snapshot(&second), enabled);
            preferences.set_omastrata_mode(false);
            assert_accelerators(&second, false);
            second.destroy();
        },
    );
}

fn policy_window() -> gtk::ApplicationWindow {
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
    window
}

fn assert_accelerators(window: &gtk::ApplicationWindow, omastrata: bool) {
    let application = window.application().expect("application");
    for (action, expected) in DEFAULT_ACCELS {
        let installed = application.accels_for_action(action);
        let suppressed = omastrata
            && matches!(
                *action,
                "win.jump-folder" | "win.open-terminal" | "win.toggle-arrow-scope"
            );
        if suppressed {
            assert!(
                installed.is_empty(),
                "{action} stays bound while Omastrata is on: {installed:?}"
            );
            continue;
        }
        assert_eq!(installed.len(), expected.len(), "{action}");
        for (actual, &expected) in installed.iter().zip(*expected) {
            assert_eq!(
                gtk::accelerator_parse(actual).expect("installed accelerator"),
                gtk::accelerator_parse(expected).expect("default accelerator"),
                "{action}"
            );
        }
    }
}

fn accel_snapshot(window: &gtk::ApplicationWindow) -> Vec<(String, Vec<String>)> {
    let application = window.application().expect("application");
    DEFAULT_ACCELS
        .iter()
        .map(|(action, _)| {
            (
                (*action).to_owned(),
                application
                    .accels_for_action(action)
                    .iter()
                    .map(|accel| accel.to_string())
                    .collect(),
            )
        })
        .collect()
}

fn settle_policy() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

#[test]
fn native_editing_shortcuts_are_left_to_the_focused_widget() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;

    for key in [
        gtk::gdk::Key::a,
        gtk::gdk::Key::c,
        gtk::gdk::Key::v,
        gtk::gdk::Key::x,
    ] {
        assert!(is_native_editing_shortcut(key, control));
    }
    assert!(!is_native_editing_shortcut(
        gtk::gdk::Key::c,
        control | gtk::gdk::ModifierType::SHIFT_MASK
    ));
}
