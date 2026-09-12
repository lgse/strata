// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use super::*;
use crate::{
    app::BrowserEvent,
    model::{EntryKind, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::gtk_test,
};

type PendingLoad = (DirectoryRequest, Rc<dyn Fn(DirectoryEvent)>);

#[derive(Default)]
struct Source {
    pending: RefCell<Option<PendingLoad>>,
}

impl FileSource for Source {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.pending.replace(Some((request, emit)));
        LoadHandle::new(|| {})
    }
}

impl Source {
    fn finish(&self, omit: Option<usize>) {
        let (request, emit) = self.pending.take().expect("pending directory");
        let root = request.location.native_path().expect("native fixture");
        let entries = (0..240)
            .filter(|position| Some(*position) != omit)
            .map(|position| {
                let name = format!("folder-{position:03}");
                FileEntry {
                    location: Location::local(root.join(&name)),
                    native_name: name.clone().into(),
                    display_name: name,
                    thumbnail_path: None,
                    kind: EntryKind::Directory,
                    size: MetadataValue::Known(0),
                    modified_unix_seconds: MetadataValue::Known(1),
                    mode: MetadataValue::Known(0o40755),
                    is_hidden: false,
                }
            })
            .collect();
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries,
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
    }
}

struct Fixture {
    source: Rc<Source>,
    browser: Rc<Browser>,
    views: Rc<RefCell<ModeViews>>,
    window: gtk::Window,
    _theme: Rc<crate::ui::theme::ThemeManager>,
}

impl Fixture {
    fn new(grouped: bool) -> Self {
        let theme = crate::ui::theme::ThemeManager::shared();
        crate::ui::prepare_portal_ui();
        let source = Rc::new(Source::default());
        let browser = Browser::new(source.clone());
        let views = Rc::new(RefCell::new(ModeViews::new(
            &gtk::ScrolledWindow::new(),
            browser.clone(),
            Rc::new(Cell::new(true)),
        )));
        views.borrow_mut().set_group_by_type(grouped);
        views.borrow_mut().prepare_mode(BrowserMode::List);
        views.borrow().show_mode(BrowserMode::List);
        let weak = Rc::downgrade(&views);
        browser.observe(move |event| {
            if !matches!(event, BrowserEvent::SelectionSynced { .. })
                && let Some(views) = weak.upgrade()
            {
                views.borrow_mut().handle(event);
            }
        });
        let window = gtk::Window::builder()
            .default_width(800)
            .default_height(600)
            .child(&views.borrow().widget())
            .build();
        window.present();
        browser.navigate(Location::local("/fixture"));
        let fixture = Self {
            source,
            browser,
            views,
            window,
            _theme: theme,
        };
        fixture.finish(None);
        fixture
    }

    fn pane(&self) -> Pane {
        self.views.borrow().list_pane.clone().expect("list pane")
    }

    fn finish(&self, omit: Option<usize>) {
        self.source.finish(omit);
        pump_until(|| {
            !self
                .browser
                .column_snapshot(0)
                .expect("active column")
                .loading
        });
        pump_until(|| self.pane().section.view.is_mapped());
        pump_until(|| !self.views.borrow().list_navigation.borrow().is_restoring());
        pump_frames(&self.pane().section.view, 8);
    }

    fn select_and_scroll(&self, position: usize) -> f64 {
        self.browser.select(0, position);
        let pane = self.pane();
        pump_frames(&pane.section.view, 8);
        let (vertical, _) = adjustments(&pane).expect("list scrollers");
        vertical.set_value((vertical.value() - 170.0).max(0.0));
        pump_frames(&pane.section.view, 8);
        assert!(vertical.value() > vertical.page_size());
        vertical.value()
    }

    fn assert_restored(&self, name: &str, vertical: f64) {
        assert_eq!(
            self.browser
                .selected_entries()
                .iter()
                .map(|entry| entry.display_name.as_str())
                .collect::<Vec<_>>(),
            [name]
        );
        let focused = self.browser.focused_item().expect("restored cursor");
        assert_eq!(focused.2.display_name, name);
        assert_eq!(self.views.borrow().focused_position(), Some((0, focused.1)));
        let (adjustment, _) = adjustments(&self.pane()).expect("list scrollers");
        assert!(
            (adjustment.value() - vertical).abs() < 2.0,
            "saved={vertical}, restored={}",
            adjustment.value()
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.browser.clear_observer();
        self.views.borrow_mut().clear_list();
        self.window.close();
    }
}

fn pump_until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "GTK state did not settle");
        glib::MainContext::default().iteration(false);
    }
}

fn pump_frames(view: &gtk::Widget, count: u8) {
    let remaining = Rc::new(Cell::new(count));
    let frames = remaining.clone();
    view.add_tick_callback(move |_, _| {
        frames.set(frames.get() - 1);
        if frames.get() == 0 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    pump_until(|| remaining.get() == 0);
}

#[test]
fn list_back_up_and_forward_restore_each_nested_viewport_and_cursor() {
    gtk_test(
        "ui::browser_modes::navigation::tests::list_back_up_and_forward_restore_each_nested_viewport_and_cursor",
        || {
            for grouped in [false, true] {
                for back in [false, true] {
                    let fixture = Fixture::new(grouped);
                    let root_scroll = fixture.select_and_scroll(140);
                    fixture.browser.activate_in_place(0, 140);
                    fixture.finish(None);
                    let child_scroll = fixture.select_and_scroll(90);
                    fixture.browser.activate_in_place(0, 90);
                    fixture.finish(None);
                    if back {
                        fixture.browser.back();
                    } else {
                        fixture.browser.parent();
                    }
                    fixture.finish(None);
                    fixture.assert_restored("folder-090", child_scroll);
                    if back {
                        fixture.browser.back();
                    } else {
                        fixture.browser.parent();
                    }
                    fixture.finish(None);
                    fixture.assert_restored("folder-140", root_scroll);
                    if back {
                        fixture.browser.forward();
                        fixture.finish(None);
                        fixture.assert_restored("folder-090", child_scroll);
                    }
                }
            }
        },
    );
}

#[test]
fn list_return_restores_multiselection_and_native_range_anchor() {
    gtk_test(
        "ui::browser_modes::navigation::tests::list_return_restores_multiselection_and_native_range_anchor",
        || {
            let fixture = Fixture::new(false);
            fixture.select_and_scroll(140);
            fixture.browser.set_selection(0, &[139, 140], Some(140));
            fixture.browser.set_selection_anchor(0, 139);
            set_selections(&fixture.pane(), &[139, 140]);
            fixture.browser.navigate(Location::local("/elsewhere"));
            fixture.finish(None);
            fixture.browser.back();
            fixture.finish(None);
            assert_eq!(fixture.browser.selected_positions(0), [139, 140]);
            assert_eq!(
                fixture.views.borrow().selected_positions(),
                Some((0, vec![139, 140]))
            );
            assert_eq!(fixture.views.borrow().focused_position(), Some((0, 140)));
            fixture
                .pane()
                .section
                .view
                .activate_action(
                    "list.select-item",
                    Some(&(141u32, false, true).to_variant()),
                )
                .expect("native range-selection action");
            assert_eq!(fixture.browser.selected_positions(0), [139, 140, 141]);
        },
    );
}

#[test]
fn list_restoration_tracks_locations_and_ignores_departed_loads() {
    gtk_test(
        "ui::browser_modes::navigation::tests::list_restoration_tracks_locations_and_ignores_departed_loads",
        || {
            let fixture = Fixture::new(false);
            let saved = fixture.select_and_scroll(140);
            fixture.browser.activate_in_place(0, 140);
            fixture.finish(None);
            fixture.browser.back();
            fixture.finish(Some(10));
            fixture.assert_restored("folder-140", saved);
            assert_eq!(
                fixture.browser.focused_item().expect("restored cursor").1,
                139
            );

            fixture.browser.activate_in_place(0, 139);
            fixture.finish(None);
            fixture.browser.back();
            // Leave before the cached visit can finish loading.
            fixture.browser.navigate(Location::local("/elsewhere"));
            fixture.finish(None);
            assert_eq!(
                fixture.browser.selected_entries()[0].display_name,
                "folder-000"
            );
            fixture.browser.navigate(Location::local("/fixture"));
            fixture.finish(Some(140));
            assert!(
                fixture.browser.selected_entries().is_empty(),
                "a deleted selection must not select its old index"
            );
            let (vertical, _) = adjustments(&fixture.pane()).expect("list scrollers");
            assert!((vertical.value() - saved).abs() < 2.0);
        },
    );
}
