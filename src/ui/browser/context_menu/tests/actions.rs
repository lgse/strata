// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser::{BrowserView, PeekBehavior};
use std::time::{Duration, Instant};

#[test]
fn selection_actions_resolve_at_click_time_and_do_not_keep_the_view_alive() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::actions::selection_actions_resolve_at_click_time_and_do_not_keep_the_view_alive",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::write(fixture.path().join("alpha"), b"alpha").expect("first file");
            std::fs::write(fixture.path().join("beta"), b"beta").expect("second file");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
            {
                assert!(Instant::now() < deadline, "listing did not finish");
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(browser.column_snapshot(0).expect("column").count, 2);
            let button = gtk::Button::new();
            let popover = gtk::Popover::new();
            let target = Rc::new(RefCell::new(None));
            let received = Rc::new(RefCell::new(Vec::new()));
            let results = received.clone();
            connect_selection_action(
                &button,
                &popover,
                &view.state,
                &target,
                move |_, entries| {
                    results.borrow_mut().push(
                        entries
                            .into_iter()
                            .map(|entry| entry.location)
                            .collect::<Vec<_>>(),
                    );
                },
            );

            let fallback = browser.entry_at(0, 0).expect("fallback entry");
            target.replace(Some((0, fallback.clone())));
            let selection = view.state.columns.borrow()[0].selection.clone();
            selection.unselect_all();
            button.emit_clicked();
            assert_eq!(received.borrow()[0], vec![fallback.location.clone()]);

            selection.select_all();
            button.emit_clicked();
            assert_eq!(
                received.borrow()[1],
                vec![
                    fallback.location,
                    browser.entry_at(0, 1).expect("second entry").location,
                ]
            );

            selection.unselect_all();
            selection.select_item(0, true);
            let direct_target = browser.entry_at(0, 1).expect("direct target");
            target.replace(Some((1, direct_target.clone())));
            button.emit_clicked();
            assert_eq!(received.borrow()[2], vec![direct_target.location]);

            selection.unselect_all();
            target.take();
            button.emit_clicked();
            assert!(received.borrow()[3].is_empty());

            let weak = Rc::downgrade(&view.state);
            browser.clear_observer();
            drop(view);
            assert!(
                weak.upgrade().is_none(),
                "menu callback must not own the browser view"
            );
            button.emit_clicked();
            assert_eq!(received.borrow().len(), 4, "a stale action must not run");
        },
    );
}
