// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::{
    services::{
        DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
        RequestId,
    },
    test_support::gtk_test,
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
    },
};
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "spinner fixture did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn settle_spinner_delay() {
    let deadline = Instant::now() + COLUMN_SPINNER_DELAY * 2;
    wait_until(|| Instant::now() >= deadline);
}

#[derive(Clone)]
struct PendingLoad {
    id: RequestId,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    cancelled: Rc<Cell<bool>>,
}

impl PendingLoad {
    fn finish(&self) {
        (self.emit)(DirectoryEvent::Finished {
            request_id: self.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
    }

    fn fail(&self) {
        (self.emit)(DirectoryEvent::Failed {
            request_id: self.id,
            message: "fixture failure".into(),
        });
    }
}

#[derive(Default)]
struct ControlledFileSource {
    loads: RefCell<Vec<PendingLoad>>,
}

impl ControlledFileSource {
    fn load(&self, index: usize) -> PendingLoad {
        wait_until(|| self.loads.borrow().len() > index);
        self.loads.borrow()[index].clone()
    }
}

impl FileSource for ControlledFileSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let cancelled = Rc::new(Cell::new(false));
        self.loads.borrow_mut().push(PendingLoad {
            id: request.id,
            emit,
            cancelled: cancelled.clone(),
        });
        LoadHandle::new(move || cancelled.set(true))
    }
}

struct TestView {
    view: BrowserView,
    window: gtk::Window,
}

impl TestView {
    fn new(source: Rc<dyn FileSource>, mode: BrowserMode) -> Self {
        let view = BrowserView::new(source, PeekBehavior::default());
        view.set_view_mode(mode);
        let window = gtk::Window::builder()
            .child(&view.widget())
            .default_width(640)
            .default_height(500)
            .build();
        window.present();
        Self { view, window }
    }

    fn assert_spinner(&self, visible: bool) {
        let columns = self.view.state.columns.borrow();
        let column = &columns[0];
        assert_eq!(column.spinner.is_visible(), visible);
        assert_eq!(column.spinner.is_spinning(), visible);
        assert!(
            column.spinner_delay.borrow().is_none(),
            "no stale delayed spinner"
        );
    }

    fn wait_for_load(&self) {
        wait_until(|| {
            self.view
                .browser()
                .column_snapshot(0)
                .is_some_and(|snapshot| !snapshot.loading)
        });
    }
}

impl Drop for TestView {
    fn drop(&mut self) {
        self.view.browser().clear_observer();
        self.window.destroy();
    }
}

#[test]
fn switching_to_columns_after_the_load_finished_does_not_leave_the_spinner_stuck() {
    gtk_test(
        "ui::browser::columns::tests::spinner::switching_to_columns_after_the_load_finished_does_not_leave_the_spinner_stuck",
        || {
            let fixture = tempfile::tempdir().expect("directory fixture");
            std::fs::write(fixture.path().join("file.txt"), "fixture").expect("fixture file");
            let test = TestView::new(Rc::new(crate::adapters::LocalFileSource), BrowserMode::List);
            test.view
                .browser()
                .navigate(Location::local(fixture.path()));
            test.wait_for_load();
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                test.view.set_view_mode(mode);
                test.view.set_view_mode(BrowserMode::Columns);
                test.assert_spinner(false);
                settle_spinner_delay();
                test.assert_spinner(false);
            }
        },
    );
}

#[test]
fn switching_to_columns_while_still_loading_shows_the_spinner_immediately() {
    gtk_test(
        "ui::browser::columns::tests::spinner::switching_to_columns_while_still_loading_shows_the_spinner_immediately",
        || {
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                let source = Rc::new(ControlledFileSource::default());
                let test = TestView::new(source.clone(), mode);
                test.view
                    .browser()
                    .navigate(Location::local("/fixture/pending"));
                let load = source.load(0);
                // Do not dispatch the delayed timer: the rebuilt spinner must be visible synchronously.
                test.view.set_view_mode(BrowserMode::Columns);
                test.assert_spinner(true);
                load.finish();
                test.wait_for_load();
                settle_spinner_delay();
                test.assert_spinner(false);
            }
        },
    );
}

#[test]
fn failed_and_empty_loads_stay_idle_after_mode_switches_and_refresh() {
    gtk_test(
        "ui::browser::columns::tests::spinner::failed_and_empty_loads_stay_idle_after_mode_switches_and_refresh",
        || {
            let source = Rc::new(ControlledFileSource::default());
            let test = TestView::new(source.clone(), BrowserMode::List);
            let browser = test.view.browser();
            browser.navigate(Location::local("/fixture/failure"));
            source.load(0).fail();
            test.wait_for_load();
            test.view.set_view_mode(BrowserMode::Columns);
            settle_spinner_delay();
            test.assert_spinner(false);
            assert!(
                browser
                    .column_snapshot(0)
                    .expect("failed column")
                    .error
                    .is_some()
            );

            browser.refresh_all();
            let refresh = source.load(1);
            test.assert_spinner(true);
            refresh.finish();
            test.wait_for_load();
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                test.view.set_view_mode(mode);
                test.view.set_view_mode(BrowserMode::Columns);
                settle_spinner_delay();
                test.assert_spinner(false);
            }
            let snapshot = browser.column_snapshot(0).expect("empty column");
            assert_eq!(snapshot.count, 0);
            assert!(snapshot.error.is_none());
        },
    );
}

#[test]
fn superseded_load_events_do_not_change_the_rebuilt_spinner() {
    gtk_test(
        "ui::browser::columns::tests::spinner::superseded_load_events_do_not_change_the_rebuilt_spinner",
        || {
            let source = Rc::new(ControlledFileSource::default());
            let test = TestView::new(source.clone(), BrowserMode::List);
            let browser = test.view.browser();
            browser.navigate(Location::local("/fixture/old"));
            let old = source.load(0);
            test.view.set_view_mode(BrowserMode::Columns);
            test.assert_spinner(true);
            browser.navigate(Location::local("/fixture/current"));
            let current = source.load(1);
            assert!(old.cancelled.get());
            test.view.set_view_mode(BrowserMode::Icons);
            test.view.set_view_mode(BrowserMode::Columns);
            old.finish();
            old.fail();
            settle_spinner_delay();
            test.assert_spinner(true);
            current.fail();
            test.wait_for_load();
            old.finish();
            settle_spinner_delay();
            test.assert_spinner(false);
            let snapshot = browser.column_snapshot(0).expect("current column");
            assert_eq!(snapshot.location, Location::local("/fixture/current"));
            assert!(snapshot.error.is_some());
        },
    );
}
