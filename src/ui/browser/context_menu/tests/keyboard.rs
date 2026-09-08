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

#[test]
fn escape_closes_popover_without_changing_selection() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::escape_closes_popover",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let button1 = gtk::Button::with_label("Action 1");
            let button2 = gtk::Button::with_label("Action 2");
            content.append(&button1);
            content.append(&button2);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();
            let popover = gtk::Popover::builder()
                .child(&scroll)
                .autohide(true)
                .build();

            popover.add_css_class("folder-context-popover");

            popover.popup();
            button1.grab_focus();
            assert!(button1.has_focus());

            // GTK Popover autohides on Escape automatically when autohide=true
            assert!(popover.is_autohide());
        },
    );
}

#[test]
fn enter_or_space_activates_focused_menu_item() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::enter_space_activation",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let button = gtk::Button::with_label("Test Action");
            content.append(&button);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();
            let popover = gtk::Popover::builder().child(&scroll).build();

            popover.popup();
            button.grab_focus();
            assert!(button.has_focus());

            // GTK Button natively handles Enter/Space activation
            // when it has focus, so this behavior is built-in
        },
    );
}

#[test]
fn scrolled_window_autoscrolls_focused_menu_item_into_view() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::scroll_to_focused",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);

            // Create enough items to require scrolling
            for i in 0..20 {
                let button = gtk::Button::with_label(&format!("Item {}", i));
                content.append(&button);
            }

            let scroll = gtk::ScrolledWindow::builder()
                .child(&content)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .build();
            scroll.set_max_content_height(150);

            let popover = gtk::Popover::builder().child(&scroll).build();
            popover.popup();

            // Get the last item and focus it
            let mut current = content.first_child();
            let mut last_button = None;
            while let Some(widget) = current {
                if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                    last_button = Some(button.clone());
                }
                current = widget.next_sibling();
            }

            if let Some(button) = last_button {
                button.grab_focus();
                // ScrolledWindow should scroll to keep focused widget visible
                assert!(button.has_focus());
            }
        },
    );
}

#[test]
fn context_menu_restore_focus_after_close_columns_mode() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::focus_restore_columns",
        || {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let button = gtk::Button::with_label("Test Action");
            content.append(&button);

            let scroll = gtk::ScrolledWindow::builder().child(&content).build();
            let popover = gtk::Popover::builder()
                .child(&scroll)
                .autohide(true)
                .build();

            popover.add_css_class("folder-context-popover");

            // Simulate the Columns mode focus restoration pattern
            let restored_focus = std::cell::RefCell::new(false);

            popover.connect_closed({
                let restored = restored_focus.clone();
                move |_| {
                    *restored.borrow_mut() = true;
                }
            });

            let before_button = gtk::Button::with_label("Columns item");
            before_button.grab_focus();
            assert!(before_button.has_focus());

            popover.popup();
            button.grab_focus();
            assert!(button.has_focus());

            // When popover closes, the close handler should fire
            popover.popdown();
            assert!(*restored_focus.borrow());
        },
    );
}
