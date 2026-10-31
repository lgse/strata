// SPDX-License-Identifier: MIT

use super::*;

fn thumbnail_size(fixture: &KeyboardFixture) -> i32 {
    widget_with_class(&fixture.view.widget(), "icons-thumbnail-scale")
        .and_downcast::<gtk::Scale>()
        .expect("thumbnail size slider")
        .value()
        .round() as i32
}

fn size_notice(fixture: &KeyboardFixture) -> Option<String> {
    widget_with_class(fixture.window.upcast_ref(), "overlay-notice")?
        .first_child()?
        .first_child()
        .and_downcast::<gtk::Label>()
        .map(|label| label.text().into())
}

fn wheel(
    fixture: &KeyboardFixture,
    zoom: &keyboard::WheelZoom,
    modifiers: ModifierType,
    delta: (f64, f64),
) -> glib::Propagation {
    zoom.handle(
        &PreferenceManager::shared(),
        &fixture.view,
        modifiers,
        gtk::gdk::ScrollUnit::Wheel,
        None,
        delta,
    )
}

#[test]
fn thumbnail_shortcuts_and_shift_wheel_step_icons_sizes_and_leave_other_views_alone() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::thumbnail_size::thumbnail_shortcuts_and_shift_wheel_step_icons_sizes_and_leave_other_views_alone",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            let location = fixture.view.browser().location_at(0);
            let control_alt = ModifierType::CONTROL_MASK | ModifierType::ALT_MASK;
            let control_shift = ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK;
            preferences.set_remember_folder_views(false);
            preferences.set_icons_thumbnail_size(64);
            fixture.view.set_view_mode(BrowserMode::Icons);
            focus_files(&fixture);
            wait_until(|| thumbnail_size(&fixture) == 64);

            for (key, expected) in [(Key::plus, 72), (Key::equal, 80), (Key::minus, 72)] {
                assert!(fixture.press(key, control_alt), "Ctrl+Alt+{key:?}");
                assert_eq!(thumbnail_size(&fixture), expected, "Ctrl+Alt+{key:?}");
                assert_eq!(
                    size_notice(&fixture),
                    Some(format!("Thumbnail size: {expected}px"))
                );
                assert_eq!(
                    preferences.icons_thumbnail_size(),
                    expected,
                    "without folder settings the size is the default"
                );
            }
            preferences.set_icons_thumbnail_size(100);
            wait_until(|| thumbnail_size(&fixture) == 100);
            assert!(fixture.press(Key::minus, control_alt));
            assert_eq!(thumbnail_size(&fixture), 96, "a size between steps");
            assert!(fixture.press(Key::_0, control_alt));
            assert_eq!(thumbnail_size(&fixture), 64, "reset to the built-in size");

            preferences.set_remember_folder_views(true);
            assert!(fixture.press(Key::plus, control_alt));
            assert_eq!(preferences.icons_size_for(location.as_ref()), 72);
            assert_eq!(preferences.icons_thumbnail_size(), 64);
            assert!(fixture.press(Key::_0, control_alt));
            wait_until(|| thumbnail_size(&fixture) == 64);
            assert_eq!(
                preferences.icons_size_for(location.as_ref()),
                64,
                "reset makes the folder follow the default again"
            );
            assert_eq!(
                size_notice(&fixture).as_deref(),
                Some("Thumbnail size: 64px")
            );

            let zoom = keyboard::WheelZoom::default();
            let text_size = preferences.text_size().root_font_px();
            assert_eq!(
                wheel(&fixture, &zoom, control_shift, (0.0, -1.0)),
                glib::Propagation::Stop
            );
            assert_eq!(thumbnail_size(&fixture), 72);
            assert_eq!(
                wheel(&fixture, &zoom, control_shift, (-1.0, 0.0)),
                glib::Propagation::Stop,
                "Shift+wheel that arrives as horizontal scrolling"
            );
            assert_eq!(thumbnail_size(&fixture), 80);
            wheel(&fixture, &zoom, control_shift, (0.0, 1.0));
            assert_eq!(thumbnail_size(&fixture), 72);
            assert_eq!(preferences.text_size().root_font_px(), text_size);
            wheel(&fixture, &zoom, ModifierType::CONTROL_MASK, (0.0, -1.0));
            assert_eq!(preferences.text_size().root_font_px(), text_size + 1);
            assert_eq!(thumbnail_size(&fixture), 72, "Ctrl+wheel resizes text only");
            assert_eq!(
                size_notice(&fixture),
                Some(format!("Text size: {}px", text_size + 1))
            );

            preferences.set_tenxer_mode(true);
            focus_files(&fixture);
            for (key, modifiers, expected) in [
                (Key::plus, ModifierType::SHIFT_MASK, 80),
                (Key::minus, ModifierType::empty(), 72),
                (Key::equal, ModifierType::empty(), 80),
            ] {
                assert!(fixture.press(key, modifiers), "10xer {key:?}");
                assert_eq!(thumbnail_size(&fixture), expected, "10xer {key:?}");
            }
            assert!(fixture.press(Key::plus, control_alt));
            assert_eq!(thumbnail_size(&fixture), 96, "10xer keeps Ctrl+Alt++");

            for mode in [BrowserMode::Columns, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                focus_files(&fixture);
                for (key, modifiers) in [
                    (Key::plus, control_alt),
                    (Key::_0, control_alt),
                    (Key::plus, ModifierType::SHIFT_MASK),
                    (Key::minus, ModifierType::empty()),
                ] {
                    fixture.press(key, modifiers);
                }
                assert_eq!(
                    wheel(&fixture, &zoom, control_shift, (0.0, -1.0)),
                    glib::Propagation::Proceed,
                    "{mode:?}"
                );
                assert_eq!(
                    preferences.icons_size_for(location.as_ref()),
                    96,
                    "{mode:?}"
                );
                assert_eq!(
                    preferences.text_size().root_font_px(),
                    text_size + 1,
                    "{mode:?}"
                );
            }
        },
    );
}
