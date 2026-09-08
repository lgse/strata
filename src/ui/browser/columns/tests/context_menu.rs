// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::Location;
use crate::test_support::gtk_test;
use crate::ui::browser::{BrowserView, PeekBehavior};
use std::time::{Duration, Instant};

fn column_for(fixture: &std::path::Path) -> (ColumnView, BrowserView, gtk::Window) {
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(640)
        .default_height(500)
        .build();
    window.present();
    let browser = view.browser();
    browser.navigate(Location::local(fixture));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !browser
        .column_snapshot(0)
        .is_some_and(|column| !column.loading)
    {
        assert!(Instant::now() < deadline, "listing did not finish");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    let allocated = Instant::now() + Duration::from_secs(5);
    while view.state.columns.borrow()[0].presentation.stack.width() == 0 {
        assert!(Instant::now() < allocated, "column did not allocate");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    let column = view.state.columns.borrow()[0].clone();
    (column, view, window)
}

#[test]
fn context_menu_target_resolves_focused_row() {
    gtk_test(
        "ui::browser::columns::tests::context_menu::context_menu_target_resolves_focused_row",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::write(fixture.path().join("alpha"), b"alpha").expect("fixture file");
            let (column, view, window) = column_for(fixture.path());

            let target = column.context_menu_target(Some(0));
            assert!(
                target.is_some(),
                "expected a trigger and point for position 0"
            );

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn context_menu_target_falls_back_to_background() {
    gtk_test(
        "ui::browser::columns::tests::context_menu::context_menu_target_falls_back_to_background",
        || {
            let fixture = tempfile::tempdir().expect("empty fixture");
            let (column, view, window) = column_for(fixture.path());

            let target = column.context_menu_target(None);
            assert!(
                target.is_some(),
                "expected the folder trigger with no position"
            );

            view.browser().clear_observer();
            window.destroy();
        },
    );
}
