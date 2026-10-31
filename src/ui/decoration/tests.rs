// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{assert_default_decoration_absent, claim_titlebar};
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
