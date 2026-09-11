// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn pointer_controls_center_pane_buttons_without_stretching() {
    crate::test_support::gtk_test(
        "ui::controls::tests::pointer_controls_center_pane_buttons_without_stretching",
        || {
            for widget in [
                gtk::Button::new().upcast::<gtk::Widget>(),
                gtk::ToggleButton::new().upcast(),
                gtk::MenuButton::new().upcast(),
            ] {
                pane_header_action(&widget);
                assert!(widget.has_css_class("column-header-action"));
                assert_eq!(widget.valign(), gtk::Align::Center);
                assert_eq!(
                    widget.cursor().and_then(|cursor| cursor.name()).as_deref(),
                    Some("pointer")
                );
            }
        },
    );
}

#[test]
fn dialog_copy_wraps_at_word_boundaries() {
    let wrapped = wrap_dialog_text(
        "Those credentials were not accepted. Check the username and password.",
        32,
    );
    assert_eq!(
        wrapped,
        "Those credentials were not\naccepted. Check the username and\npassword."
    );
    assert!(wrapped.lines().all(|line| line.chars().count() <= 32));
}

#[test]
fn dialog_copy_wraps_long_paths_without_spaces() {
    let wrapped = wrap_dialog_text(&"a".repeat(80), 32);
    assert_eq!(
        wrapped.lines().map(str::len).collect::<Vec<_>>(),
        [32, 32, 16]
    );
}
