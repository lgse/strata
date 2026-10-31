// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use gtk::glib;

use super::*;
use crate::{
    adapters::LocalFileSource,
    model::Location,
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, MetadataRequest, Responsiveness, StampAnswer, Watching,
    },
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
        preferences::PreferenceManager,
    },
};

struct Mount {
    responsiveness: Rc<Cell<Responsiveness>>,
}

impl FileSource for Mount {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        LocalFileSource.validate_location_async(location, emit)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        LocalFileSource.enumerate(request, emit)
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        LocalFileSource.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        LocalFileSource.fill_metadata(request, emit)
    }

    fn start_watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
        report: Rc<dyn Fn(Watching)>,
    ) -> LoadHandle {
        LocalFileSource.start_watch(location, include_hidden, notify, report)
    }

    fn probe_stamp(&self, location: Location, emit: Rc<dyn Fn(StampAnswer)>) -> LoadHandle {
        LocalFileSource.probe_stamp(location, emit)
    }

    fn responsiveness(&self, _location: &Location) -> Responsiveness {
        self.responsiveness.get()
    }
}

fn pump_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn notice_drawn(root: &gtk::Widget) -> bool {
    let text = crate::i18n::tr("Not responding. This list may be out of date.");
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == text && label.is_drawable())
        {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            pending.push(next);
        }
    }
    false
}

#[test]
fn the_view_shows_the_not_responding_notice_in_every_mode() {
    crate::test_support::gtk_test(
        "ui::browser::presentation::tests::the_view_shows_the_not_responding_notice_in_every_mode",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("folder");
            std::fs::write(root.path().join("note.txt"), b"note").expect("entry");
            let responsiveness = Rc::new(Cell::new(Responsiveness::Answering));
            let view = BrowserView::new(
                Rc::new(Mount {
                    responsiveness: responsiveness.clone(),
                }),
                PeekBehavior::default(),
            );
            let window = gtk::Window::builder()
                .default_width(1000)
                .default_height(650)
                .child(&view.widget())
                .build();
            window.present();
            let browser = view.browser();
            browser.navigate(Location::local(root.path()));
            pump_until(
                || {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
                },
                "the folder to load",
            );
            let drawn = || {
                for _ in 0..20 {
                    glib::MainContext::default().iteration(false);
                }
                notice_drawn(window.upcast_ref())
            };

            let mut failures = Vec::new();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                view.set_view_mode(mode);
                let answering = drawn();
                responsiveness.set(Responsiveness::NotResponding);
                browser.follow_responsiveness();
                let not_responding = drawn();
                responsiveness.set(Responsiveness::Answering);
                browser.follow_responsiveness();
                let answering_again = drawn();
                let got = (answering, not_responding, answering_again);
                if got != (false, true, false) {
                    failures.push(format!("{mode:?}: drawn {got:?}"));
                }
            }
            responsiveness.set(Responsiveness::NotResponding);
            browser.follow_responsiveness();
            for mode in [BrowserMode::List, BrowserMode::Columns, BrowserMode::Icons] {
                view.set_view_mode(mode);
                if !drawn() {
                    failures.push(format!("{mode:?}: not drawn after switching to it"));
                }
            }

            PreferenceManager::shared().release_bindings_within(&view.widget());
            browser.clear_observer();
            window.destroy();
            assert!(failures.is_empty(), "{}", failures.join("\n"));
        },
    );
}
