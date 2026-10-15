// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{
    ControlLayout,
    WindowControl::{Close, Maximize, Minimize},
    assert_default_decoration_absent, claim_titlebar, control_layout,
};
use crate::test_support::gtk_test;

#[test]
fn claimed_windows_are_client_decorated_without_a_default_title_bar() {
    gtk_test(
        "ui::decoration::tests::claimed_windows_are_client_decorated_without_a_default_title_bar",
        || {
            let window = gtk::Window::new();
            claim_titlebar(&window);
            window.present();
            for _ in 0..20 {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert_default_decoration_absent(&window);
            window.close();
        },
    );
}

#[test]
fn decoration_layout_assigns_each_control_a_side() {
    for (layout, start, end) in [
        ("close:appmenu", vec![Close], vec![Minimize, Maximize]),
        (
            "appmenu:minimize,maximize,close",
            vec![],
            vec![Minimize, Maximize, Close],
        ),
        (
            "close,minimize,maximize:",
            vec![Close, Minimize, Maximize],
            vec![],
        ),
        ("appmenu:close", vec![], vec![Minimize, Maximize, Close]),
        (
            "icon, close :maximize",
            vec![],
            vec![Minimize, Close, Maximize],
        ),
        ("close", vec![Close], vec![Minimize, Maximize]),
        ("close:close", vec![Close], vec![Minimize, Maximize]),
        ("", vec![], vec![Minimize, Maximize, Close]),
    ] {
        assert_eq!(
            control_layout(layout),
            ControlLayout { start, end },
            "layout {layout:?}"
        );
    }
}
