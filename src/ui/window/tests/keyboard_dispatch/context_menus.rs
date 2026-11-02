// SPDX-License-Identifier: MIT

use super::*;

/// Hyper, Meta and Caps Lock are not shortcut modifiers.
const OPENS: [(Key, ModifierType); 8] = [
    (Key::F10, ModifierType::SHIFT_MASK),
    (Key::Menu, ModifierType::empty()),
    (
        Key::F10,
        ModifierType::SHIFT_MASK.union(ModifierType::HYPER_MASK),
    ),
    (Key::Menu, ModifierType::HYPER_MASK),
    (
        Key::F10,
        ModifierType::SHIFT_MASK.union(ModifierType::META_MASK),
    ),
    (Key::Menu, ModifierType::META_MASK),
    (
        Key::F10,
        ModifierType::SHIFT_MASK.union(ModifierType::LOCK_MASK),
    ),
    (Key::Menu, ModifierType::LOCK_MASK),
];

const NEAR_MISSES: [(Key, ModifierType); 7] = [
    (Key::F10, ModifierType::empty()),
    (
        Key::F10,
        ModifierType::SHIFT_MASK.union(ModifierType::CONTROL_MASK),
    ),
    (
        Key::F10,
        ModifierType::SHIFT_MASK.union(ModifierType::ALT_MASK),
    ),
    (
        Key::F10,
        ModifierType::SHIFT_MASK.union(ModifierType::SUPER_MASK),
    ),
    (Key::Menu, ModifierType::SHIFT_MASK),
    (Key::Menu, ModifierType::CONTROL_MASK),
    (Key::Menu, ModifierType::SUPER_MASK),
];

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
                .place_rows
                .borrow()
                .iter()
                .rev()
                .find(|(location, _)| *location == Location::uri("trash:///"))
                .map(|(_, row)| row.clone())
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
                    for (key, modifiers) in OPENS {
                        assert!(row.grab_focus());
                        assert!(fixture.press(key, modifiers), "{modifiers:?}+{key:?}");
                        wait_until(|| popover.is_visible());
                        popover.popdown();
                        wait_until(|| !popover.is_visible());
                    }
                    for (key, modifiers) in NEAR_MISSES {
                        row.grab_focus();
                        fixture.press(key, modifiers);
                        assert!(!popover.is_visible(), "{modifiers:?}+{key:?}");
                    }
                }
            }
        },
    );
}

fn visible_popovers(widget: &gtk::Widget) -> Vec<gtk::Popover> {
    let mut found: Vec<gtk::Popover> = widget
        .downcast_ref::<gtk::Popover>()
        .filter(|popover| popover.is_visible())
        .into_iter()
        .cloned()
        .collect();
    for child in children(widget) {
        found.extend(visible_popovers(&child));
    }
    found
}

/// GTK offers a key the window's capture controller declines to the focused
/// widget's own controllers.
fn deliver_to(widget: &gtk::Widget, key: Key, modifiers: ModifierType) {
    let controllers = widget.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(keys) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
        }
    }
}

/// The rendered Back button; a view-mode switch leaves the old header until it rebuilds.
fn mapped_back(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if !widget.is_mapped() {
        return None;
    }
    if widget.has_css_class("list-navigation") {
        return widget
            .first_child()
            .filter(|back| back.is_mapped() && back.width() > 0);
    }
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
        .find_map(|child| mapped_back(&child))
}

#[test]
fn context_menu_keys_on_back_list_the_back_history() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::context_menus::context_menu_keys_on_back_list_the_back_history",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let folder = Location::local(fixture._directory.path().join("alpha"));
            std::fs::create_dir(fixture._directory.path().join("alpha")).expect("folder");
            browser.navigate(folder.clone());
            wait_until(|| browser.active_location().as_ref() == Some(&folder));
            wait_loaded(&browser, 0);
            for mode in [BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                for (key, modifiers) in &OPENS[..2] {
                    let case = format!("{mode:?} {modifiers:?}+{key:?}");
                    let back = RefCell::new(None);
                    wait_until(|| {
                        back.replace(mapped_back(fixture.window.upcast_ref()));
                        back.borrow().is_some()
                    });
                    let back = back
                        .take()
                        .unwrap_or_else(|| panic!("{case}: no Back button"));
                    let history = widget_with_class(&back, "back-history-popover")
                        .and_downcast::<gtk::Popover>()
                        .unwrap_or_else(|| panic!("{case}: Back has no history menu"));
                    assert!(back.grab_focus(), "{case}: Back took no focus");
                    assert!(
                        !fixture.press(*key, *modifiers),
                        "{case}: the window claimed the key from Back"
                    );
                    deliver_to(&back, *key, *modifiers);
                    wait_until(|| history.is_visible());
                    let others = visible_popovers(fixture.window.upcast_ref())
                        .into_iter()
                        .filter(|popover| popover != &history)
                        .count();
                    assert_eq!(others, 0, "{case}: another menu opened with the history");
                    assert_eq!(
                        browser.active_location().as_ref(),
                        Some(&folder),
                        "{case}: the key went back"
                    );
                    history.popdown();
                    wait_until(|| !history.is_visible());
                }
            }
        },
    );
}
