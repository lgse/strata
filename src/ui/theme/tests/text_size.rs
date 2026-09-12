// SPDX-License-Identifier: MIT

use crate::ui::theme::{TextSize, ThemeManager};
use gtk::{gdk, glib, prelude::*};

fn settle() {
    let main_loop = glib::MainLoop::new(None, false);
    let stop = main_loop.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || stop.quit());
    main_loop.run();
}

#[test]
fn custom_text_size_loads_and_updates_two_windows_and_new_content_with_desktop_scaling() {
    crate::test_support::gtk_test(
        "ui::theme::tests::text_size::custom_text_size_loads_and_updates_two_windows_and_new_content_with_desktop_scaling",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let manager = ThemeManager::shared();
            crate::ui::prepare_portal_ui();
            let settings = gtk::Settings::default().expect("GTK settings");
            settings.set_gtk_xft_dpi(96 * 1024);
            let labels = [
                gtk::Label::new(Some("Text size")),
                gtk::Label::new(Some("Text size")),
            ];
            let windows = labels
                .each_ref()
                .map(|label| gtk::Window::builder().child(label).build());
            for window in &windows {
                window.present();
            }
            settle();
            for label in &labels {
                assert_eq!(
                    label
                        .pango_context()
                        .font_description()
                        .expect("startup font")
                        .size(),
                    24 * gtk::pango::SCALE
                );
            }
            for dpi in [96, 120, 144, 192, 96] {
                settings.set_gtk_xft_dpi(dpi * 1024);
                for pixels in [32, 11, 48, 13] {
                    manager.set_text_size(TextSize::new(pixels));
                    let rebuilt = gtk::Label::new(Some("Rebuilt view"));
                    windows[1].set_child(Some(&rebuilt));
                    settle();
                    let expected =
                        (pixels as f64 * dpi as f64 / 96.0).round() as i32 * gtk::pango::SCALE;
                    for label in [&labels[0], &rebuilt] {
                        assert_eq!(
                            label
                                .pango_context()
                                .font_description()
                                .expect("updated font")
                                .size(),
                            expected,
                            "{pixels}px at {dpi} DPI"
                        );
                    }
                    let icon = crate::assets::chrome_icon(crate::assets::icons::SEARCH);
                    assert_eq!(
                        icon.pixel_size(),
                        (16.0 * manager.interface_scale()).round() as i32
                    );
                }
            }
            for window in &windows {
                window.close();
            }
        },
    );
}

#[test]
fn custom_text_size_shortcuts_accept_standard_and_keypad_keys_without_stealing_alt_combinations() {
    use gdk::{Key, ModifierType as M};
    let size = TextSize::new(24);
    for key in [Key::plus, Key::equal, Key::KP_Add] {
        assert_eq!(
            size.for_shortcut(key, M::CONTROL_MASK),
            Some(TextSize::new(25))
        );
        assert_eq!(
            size.for_shortcut(key, M::CONTROL_MASK | M::SHIFT_MASK),
            Some(TextSize::new(25))
        );
    }
    for key in [Key::minus, Key::KP_Subtract] {
        assert_eq!(
            size.for_shortcut(key, M::CONTROL_MASK),
            Some(TextSize::new(23))
        );
    }
    for key in [Key::_0, Key::KP_0] {
        assert_eq!(
            size.for_shortcut(key, M::CONTROL_MASK),
            Some(TextSize::default())
        );
    }
    for modifiers in [
        M::empty(),
        M::SHIFT_MASK,
        M::CONTROL_MASK | M::ALT_MASK,
        M::CONTROL_MASK | M::SUPER_MASK,
    ] {
        assert_eq!(size.for_shortcut(Key::plus, modifiers), None);
    }
}
