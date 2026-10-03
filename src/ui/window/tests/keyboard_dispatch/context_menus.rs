// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn sidebar_context_shortcuts_open_place_trash_and_device_menus() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::context_menus::sidebar_context_shortcuts_open_place_trash_and_device_menus",
        || {
            let fixture = KeyboardFixture::new();
            let state = &fixture.sidebar.state;
            let pinned = state.append_pinned_place(
                0,
                "Pinned folder",
                Location::local(fixture._directory.path()),
            );
            state.append_trash_place();
            let trash = state
                .places_for_test()
                .last_child()
                .and_downcast::<gtk::Button>()
                .expect("trash row");
            let device = sidebar_button(crate::assets::icons::HARD_DRIVE, "Device");
            attach_device_actions_menu(
                &device,
                &fixture.view,
                DeviceRowActions {
                    encrypted: None,
                    release: Some(MediaRelease::UnmountMount),
                },
                None,
                Some(Rc::new(|| {})),
                None,
                None,
            );
            state.places_for_test().append(&device);
            let preferences = PreferenceManager::shared();
            fixture.shortcuts.bind_preferences(&preferences);
            for tenxer in [false, true] {
                preferences.set_tenxer_mode(tenxer);
                pump(50);
                for row in [&pinned, &trash, &device] {
                    let popover = widget_with_class(row.upcast_ref(), "folder-context-popover")
                        .expect("row context menu")
                        .downcast::<gtk::Popover>()
                        .expect("popover");
                    for (key, modifiers) in [
                        (Key::F10, ModifierType::SHIFT_MASK),
                        (Key::Menu, ModifierType::empty()),
                    ] {
                        assert!(row.grab_focus());
                        assert!(fixture.press(key, modifiers));
                        wait_until(|| popover.is_visible());
                        popover.popdown();
                        wait_until(|| !popover.is_visible());
                    }
                    row.grab_focus();
                    fixture.press(
                        Key::F10,
                        ModifierType::SHIFT_MASK | ModifierType::CONTROL_MASK,
                    );
                    assert!(!popover.is_visible());
                }
            }
        },
    );
}
