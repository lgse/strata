// SPDX-License-Identifier: GPL-3.0-or-later

use super::menus::{MenuSource, descendants, label, open_menu, wait_until};
use crate::model::Location;
use crate::ui::browser::{BrowserView, PeekBehavior};
use crate::ui::browser_modes::BrowserMode;
use crate::ui::window::keyboard::focus::focus_first_or_last_menu_item;
use gtk::prelude::*;
use std::rc::Rc;

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

/// The item context menu nests real action buttons one level inside `single`/
/// `multiple` sub-boxes (see `install_item_context_menu`); Home/End must
/// recurse into that structure rather than stop at the first non-focusable
/// direct child (`header`), which is the exact bug this traversal fixes.
#[test]
fn home_and_end_focus_real_first_and_last_menu_buttons() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::home_and_end_focus_real_first_and_last_menu_buttons",
        || {
            let (_fixture, view, window) = open_fixture(BrowserMode::Columns);

            let menu = open_menu(&view, Some("notes.txt"));

            focus_first_or_last_menu_item(&menu, true);
            let first = gtk::prelude::RootExt::focus(&window).expect("home focuses an item");
            assert!(
                descendants(&first).iter().any(|widget| widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|text| text.text() == "Open")),
                "Home must reach the first real action button, not the non-focusable header"
            );

            focus_first_or_last_menu_item(&menu, false);
            let last = gtk::prelude::RootExt::focus(&window).expect("end focuses an item");
            assert!(
                descendants(&last).iter().any(|widget| widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|text| text.text() == "Permanently delete")),
                "End must reach the last real action button inside the nested `single` box"
            );

            menu.popdown();
            wait_until(|| menu.parent().is_none());
            close_fixture(&view, &window);
        },
    );
}

/// Builds a real browser fixture with one directory of entries, in the given
/// mode, and waits until its items are mapped.
fn open_fixture(mode: BrowserMode) -> (tempfile::TempDir, BrowserView, gtk::Window) {
    let fixture = tempfile::tempdir().expect("fixture dir");
    let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.set_view_mode(mode);
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(1000)
        .default_height(850)
        .build();
    window.present();
    view.browser().navigate(Location::local(fixture.path()));
    wait_until(|| label(&view.widget(), "notes.txt").is_some());
    (fixture, view, window)
}

fn close_fixture(view: &BrowserView, window: &gtk::Window) {
    view.browser().clear_observer();
    window.destroy();
}

#[test]
fn escape_closes_popover_without_changing_selection() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::escape_closes_popover_without_changing_selection",
        || {
            let (_fixture, view, window) = open_fixture(BrowserMode::Columns);
            view.browser().select(0, 0);

            let menu = open_menu(&view, Some("notes.txt"));
            let before = view.browser().selected_entries();

            // GTK's native autohide calls popdown() on Escape; exercise the same path.
            menu.popdown();
            wait_until(|| menu.parent().is_none());

            assert!(!menu.is_visible());
            assert_eq!(view.browser().selected_entries(), before);

            close_fixture(&view, &window);
        },
    );
}

#[test]
fn enter_or_space_activates_focused_menu_item() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::enter_or_space_activates_focused_menu_item",
        || {
            let (_fixture, view, window) = open_fixture(BrowserMode::Columns);

            let menu = open_menu(&view, None);
            let select_all = descendants(menu.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| {
                    descendants(button.upcast_ref()).iter().any(|child| {
                        child
                            .downcast_ref::<gtk::Label>()
                            .is_some_and(|text| text.text() == "Select All")
                    })
                })
                .expect("Select All action");

            select_all.grab_focus();
            assert!(select_all.has_focus());

            // GTK activates a focused button's `clicked` signal on Enter/Space by
            // calling `activate()`; invoke it the same way and check the real effect.
            assert!(select_all.activate());
            wait_until(|| menu.parent().is_none());
            assert_eq!(view.browser().selected_entries().len(), 5);

            close_fixture(&view, &window);
        },
    );
}

#[test]
fn scrolled_window_autoscrolls_focused_menu_item_into_view() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::scrolled_window_autoscrolls_focused_menu_item_into_view",
        || {
            let (_fixture, view, window) = open_fixture(BrowserMode::Columns);

            let menu = open_menu(&view, Some("notes.txt"));
            let scroll = menu
                .child()
                .and_downcast::<gtk::ScrolledWindow>()
                .expect("context menu scroll");
            // Force real overflow, independent of window height or click position.
            scroll.set_propagate_natural_height(false);
            scroll.set_max_content_height(120);
            wait_until(|| scroll.vadjustment().upper() > scroll.vadjustment().page_size());

            focus_first_or_last_menu_item(&menu, false);
            let focused =
                gtk::prelude::RootExt::focus(&window).expect("last menu item takes focus");
            wait_until(|| {
                let scroll_bounds = scroll.compute_bounds(&window);
                let item_bounds = focused.compute_bounds(&window);
                let (Some(scroll_bounds), Some(item_bounds)) = (scroll_bounds, item_bounds) else {
                    return false;
                };
                item_bounds.y() >= scroll_bounds.y() - 0.5
                    && item_bounds.y() + item_bounds.height()
                        <= scroll_bounds.y() + scroll_bounds.height() + 0.5
            });

            menu.popdown();
            wait_until(|| menu.parent().is_none());
            close_fixture(&view, &window);
        },
    );
}

/// The real path: select an item (which takes real GTK keyboard focus), open its
/// context menu the way a keyboard invocation does, close it the way GTK's own
/// Escape-autohide does, and confirm focus actually lands back on the same widget.
fn assert_restores_focus_after_keyboard_close(mode: BrowserMode) {
    let (_fixture, view, window) = open_fixture(mode);
    view.browser().select(0, 0);
    wait_until(|| gtk::prelude::RootExt::focus(&window).is_some());
    let originating = gtk::prelude::RootExt::focus(&window).expect("item takes focus");

    assert!(view.open_focused_context_menu());
    let root = window.clone().upcast::<gtk::Widget>();
    let popover = std::cell::RefCell::new(None);
    wait_until(|| {
        popover.replace(
            descendants(&root)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Popover>().ok())
                .find(|popover| popover.is_visible()),
        );
        popover.borrow().is_some()
    });
    let popover = popover.into_inner().expect("context menu popover");

    popover.popdown();
    wait_until(|| popover.parent().is_none());

    let restored = gtk::prelude::RootExt::focus(&window);
    assert_eq!(
        restored.as_ref(),
        Some(&originating),
        "{mode:?}: focus must return to the originating item after a keyboard-closed menu"
    );

    close_fixture(&view, &window);
}

#[test]
fn context_menu_restore_focus_after_close_columns_mode() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::context_menu_restore_focus_after_close_columns_mode",
        || assert_restores_focus_after_keyboard_close(BrowserMode::Columns),
    );
}

#[test]
fn context_menu_restore_focus_after_close_icons_mode() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::context_menu_restore_focus_after_close_icons_mode",
        || assert_restores_focus_after_keyboard_close(BrowserMode::Icons),
    );
}
