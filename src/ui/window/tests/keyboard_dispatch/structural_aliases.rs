// SPDX-License-Identifier: MIT

//! Extra keys bound to the structural commands, as a saved keymap binds them.

use super::*;
use crate::ui::keys::{
    Binding, Command, Keymap, Map,
    bindings::{DOWN, ENTER, FOCUS_NEXT},
    install_native_aliases, use_keymap,
};

/// `command` in both maps, with `extra` after its shipped keys.
fn with_extra(keymap: Keymap, command: Command, extra: Binding) -> Keymap {
    [Map::Default, Map::Tenxer]
        .into_iter()
        .fold(keymap, |keymap, map| {
            let mut bindings = command.defaults(map).to_vec();
            bindings.push(extra);
            keymap.with(map, command, bindings)
        })
}

#[test]
fn aliased_arrow_reaches_gtk_list() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::structural_aliases::aliased_arrow_reaches_gtk_list",
        || {
            let _keymap = use_keymap(with_extra(Keymap::defaults(), DOWN, Binding::ctrl(Key::j)));
            for tenxer in [false, true] {
                let fixture = KeyboardFixture::new();
                PreferenceManager::shared().set_tenxer_mode(tenxer);
                wait_until(|| focused_widget_shows(&fixture.window, "a.txt"));
                assert!(fixture.press(Key::j, ModifierType::CONTROL_MASK));
                wait_until(|| focused_widget_shows(&fixture.window, "b.txt"));
                assert!(fixture.view.item_view_has_focus(), "tenxer={tenxer}");
                PreferenceManager::shared().set_tenxer_mode(false);
            }
        },
    );
}

#[test]
fn aliased_tab_and_enter_synthesize() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::structural_aliases::aliased_tab_and_enter_synthesize",
        || {
            let keymap = with_extra(Keymap::defaults(), FOCUS_NEXT, Binding::plain(Key::F7));
            let _keymap = use_keymap(with_extra(keymap, ENTER, Binding::plain(Key::F8)));
            let fixture = KeyboardFixture::new();
            let native = install_native_aliases(&fixture.window);
            let press = |key: Key| {
                fixture.press(key, ModifierType::empty())
                    || native
                        .emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::empty()])
            };
            let sidebar_focused = || {
                crate::ui::focus_navigation::contains_widget(
                    &fixture.sidebar.widget,
                    focus(&fixture).as_ref(),
                )
            };

            fixture.sidebar.state.focus_active_place();
            wait_until(sidebar_focused);
            tab(&fixture, gtk::DirectionType::TabForward);
            let after_tab = focus(&fixture);
            assert!(!sidebar_focused(), "Tab stayed in the sidebar");
            fixture.sidebar.state.focus_active_place();
            wait_until(sidebar_focused);
            assert!(press(Key::F7));
            pump(50);
            assert_eq!(
                focus(&fixture),
                after_tab,
                "the Tab alternate moved focus elsewhere"
            );

            let active = fixture.sidebar_toggle.is_active();
            assert!(fixture.sidebar_toggle.grab_focus());
            assert!(press(Key::F8));
            // GTK emits a keyboard-activated button's click after its press animation.
            wait_until(|| fixture.sidebar_toggle.is_active() != active);

            let sub = fixture._directory.path().join("sub");
            std::fs::create_dir(&sub).expect("subfolder");
            let browser = fixture.view.browser();
            fixture.view.refresh();
            wait_until(|| entry_count(&browser) == 4);
            select_named(&fixture, "sub");
            wait_until(|| focused_widget_shows(&fixture.window, "sub"));
            assert!(press(Key::F8));
            wait_until(|| location_ends_with(browser.active_location(), "sub"));
        },
    );
}

fn focus(fixture: &KeyboardFixture) -> Option<gtk::Widget> {
    gtk::prelude::RootExt::focus(&fixture.window)
}
