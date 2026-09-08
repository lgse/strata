// SPDX-License-Identifier: GPL-3.0-or-later

use crate::ui::window::keyboard::focus::focus_first_or_last_menu_item;
use gtk::prelude::*;

#[test]
fn focus_first_or_last_menu_item_does_not_crash_with_no_popover_child() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::focus_first_or_last_menu_item_does_not_crash_with_no_popover_child",
        || {
            let popover = gtk::Popover::new();
            // Should not crash even if popover has no child
            focus_first_or_last_menu_item(&popover, true);
            focus_first_or_last_menu_item(&popover, false);
        },
    );
}

#[test]
fn focus_first_or_last_menu_item_does_not_crash_with_empty_menu() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::focus_first_or_last_menu_item_does_not_crash_with_empty_menu",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();

            let popover = gtk::Popover::builder().child(&scroll).build();

            // Should not crash with empty menu
            focus_first_or_last_menu_item(&popover, true);
            focus_first_or_last_menu_item(&popover, false);
        },
    );
}

#[test]
fn focus_first_or_last_menu_item_does_not_crash_with_only_separators() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::focus_first_or_last_menu_item_does_not_crash_with_only_separators",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let separator1 = gtk::Separator::new(gtk::Orientation::Horizontal);
            let separator2 = gtk::Separator::new(gtk::Orientation::Horizontal);

            content.append(&separator1);
            content.append(&separator2);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();

            let popover = gtk::Popover::builder().child(&scroll).build();

            // Should not crash with only separators
            focus_first_or_last_menu_item(&popover, true);
            focus_first_or_last_menu_item(&popover, false);
        },
    );
}

#[test]
fn focus_first_or_last_menu_item_does_not_crash_with_mixed_disabled_and_separators() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::focus_first_or_last_menu_item_does_not_crash_with_mixed_disabled_and_separators",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            let disabled_button = gtk::Button::with_label("Disabled");
            disabled_button.set_sensitive(false);

            content.append(&separator);
            content.append(&disabled_button);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();

            let popover = gtk::Popover::builder().child(&scroll).build();

            // Should not crash
            focus_first_or_last_menu_item(&popover, true);
            focus_first_or_last_menu_item(&popover, false);
        },
    );
}

#[test]
fn focus_first_or_last_menu_item_does_not_crash_with_enabled_items() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::focus_first_or_last_menu_item_does_not_crash_with_enabled_items",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let first_button = gtk::Button::with_label("First");
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            let last_button = gtk::Button::with_label("Last");

            content.append(&first_button);
            content.append(&separator);
            content.append(&last_button);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();

            let popover = gtk::Popover::builder().child(&scroll).build();

            // Should not crash and should successfully navigate to enabled items
            focus_first_or_last_menu_item(&popover, true);
            focus_first_or_last_menu_item(&popover, false);
        },
    );
}
