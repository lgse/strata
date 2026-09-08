// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::Location;
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "browser did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn open_focused_context_menu_in_columns_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::context_menu::open_focused_context_menu_in_columns_mode",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            std::fs::write(fixture.path().join("item.txt"), "content").expect("fixture file");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
            browser.select(0, 0);
            browser.focus_active();
            assert_eq!(view.view_mode(), BrowserMode::Columns);
            assert!(view.open_focused_context_menu());
            view.browser().clear_observer();
            window.close();
        },
    );
}

#[test]
fn open_focused_context_menu_in_icons_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::context_menu::open_focused_context_menu_in_icons_mode",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            std::fs::write(fixture.path().join("item.txt"), "content").expect("fixture file");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
            view.set_view_mode(BrowserMode::Icons);
            wait_until(|| view.view_mode() == BrowserMode::Icons);
            browser.select(0, 0);
            browser.focus_active();
            assert!(view.open_focused_context_menu());
            view.browser().clear_observer();
            window.close();
        },
    );
}

#[test]
fn open_focused_context_menu_opens_background_menu_when_no_item_focused() {
    crate::test_support::gtk_test(
        "ui::browser::tests::context_menu::open_focused_context_menu_opens_background_menu_when_no_item_focused",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| browser.column_snapshot(0).is_some_and(|s| !s.loading));
            let column = view.state.columns.borrow()[0].clone();
            wait_until(|| {
                column.presentation.stack.width() > 0 && column.presentation.stack.height() > 0
            });
            // No item selected/focused: the active pane's background menu should
            // still open, anchored via `Browser::active_depth()`.
            assert!(view.open_focused_context_menu());
            view.browser().clear_observer();
            window.close();
        },
    );
}

#[test]
fn open_focused_context_menu_returns_false_with_no_active_pane() {
    crate::test_support::gtk_test(
        "ui::browser::tests::context_menu::open_focused_context_menu_returns_false_with_no_active_pane",
        || {
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(900)
                .default_height(500)
                .build();
            window.present();
            // Never navigated: no column exists yet, so there is no focused item
            // and no active depth to fall back to.
            assert!(!view.open_focused_context_menu());
            view.browser().clear_observer();
            window.close();
        },
    );
}
