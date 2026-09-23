// SPDX-License-Identifier: MIT

use super::super::*;

#[test]
fn custom_text_size_shortcuts_accept_standard_and_keypad_keys_without_stealing_alt_combinations() {
    use gtk::gdk::{Key, ModifierType as M};
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

#[test]
fn root_font_size_snaps_to_a_whole_effective_pixel() {
    let scale_factor = 13.0 / 11.0;
    let root_font_px = snapped_root_font_px(15, scale_factor);

    assert_eq!(root_font_px, 18.0);
}

#[test]
fn root_font_size_is_unchanged_without_desktop_scaling() {
    let size = TextSize::new(15);
    assert_eq!(
        snapped_root_font_px(size.root_font_px(), 1.0),
        f64::from(size.root_font_px())
    );
}

#[test]
fn invalid_scaling_values_leave_the_root_font_size_unchanged() {
    for scale_factor in [0.0, -1.0, f64::NAN] {
        assert_eq!(snapped_root_font_px(15, scale_factor), 15.0);
    }
}

#[test]
fn xft_dpi_converts_to_desktop_text_scale() {
    assert_eq!(text_scale_factor_from_xft_dpi(-1), 1.0);
    assert_eq!(text_scale_factor_from_xft_dpi(96 * 1024), 1.0);
    assert!((text_scale_factor_from_xft_dpi(115_200) - 1.171_875).abs() < f64::EPSILON);
}
