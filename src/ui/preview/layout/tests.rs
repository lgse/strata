// SPDX-License-Identifier: MIT

mod visibility;

use super::super::tests::media_size::{RecordingProvider, entry, wait_until};
use super::*;
use crate::{model::Location, ui::theme::ThemeManager};

#[test]
fn automatic_and_manual_widths_reserve_space_without_losing_the_session_choice() {
    let geometry = Geometry {
        available: 2000,
        occupied: 500,
        start_minimum: 500,
        separator: 2,
        columns: true,
    };
    assert_eq!(geometry.position(None), geometry.occupied);
    let overflow = Geometry {
        occupied: 1800,
        ..geometry
    };
    assert_eq!(
        overflow.preview_width(None),
        COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER
    );
    assert_eq!(overflow.preview_width(Some(450)), 450);
    assert_eq!(geometry.preview_width(Some(450)), 450);
    let narrow = Geometry {
        available: 900,
        ..geometry
    };
    assert!(narrow.can_show_preview());
    assert_eq!(narrow.preview_width(None), 398);
    assert_eq!(narrow.position(Some(900)), narrow.start_minimum);
    assert!(
        !Geometry {
            available: 800,
            ..geometry
        }
        .can_show_preview()
    );
    assert!(
        Geometry {
            available: 802,
            ..geometry
        }
        .can_show_preview()
    );
    assert_eq!(geometry.preview_width(Some(900)), 900);
}

fn find(root: &impl IsA<gtk::Widget>, class: &str) -> Option<gtk::Widget> {
    let root = root.as_ref();
    if root.has_css_class(class) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

struct Fixture {
    window: gtk::Window,
    split: gtk::Paned,
    content: gtk::Paned,
    browser: BrowserView,
    preview: PreviewDrawer,
    requests: Rc<RefCell<Vec<PreviewRequest>>>,
    root: tempfile::TempDir,
}

impl Fixture {
    fn new(chooser: bool) -> Self {
        crate::ui::prepare_portal_ui();
        let root = tempfile::tempdir().expect("column fixture");
        std::fs::create_dir_all(root.path().join("child/grandchild")).expect("nested folders");
        let browser = if chooser {
            BrowserView::new_chooser(Rc::new(crate::adapters::LocalFileSource), false)
        } else {
            BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                Default::default(),
            )
        };
        let requests = Rc::new(RefCell::new(Vec::new()));
        let preview = PreviewDrawer::new(Rc::new(RecordingProvider(requests.clone())), !chooser);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_width_request(180);
        let content = gtk::Paned::new(gtk::Orientation::Horizontal);
        content.set_start_child(Some(&sidebar));
        content.set_end_child(Some(&browser.widget()));
        content.set_position(180);
        content.set_resize_start_child(false);
        content.set_shrink_start_child(false);
        let split = gtk::Paned::new(gtk::Orientation::Horizontal);
        split.set_start_child(Some(&content));
        split.set_resize_start_child(true);
        split.set_resize_end_child(false);
        split.set_shrink_start_child(false);
        split.set_shrink_end_child(true);
        preview.attach_split(&split, &content, &browser);
        let window = gtk::Window::builder()
            .child(&split)
            .default_width(1800)
            .default_height(800)
            .build();
        window.set_width_request(1800);
        window.present();
        browser.browser().navigate(Location::local(root.path()));
        wait_until(|| {
            browser
                .browser()
                .column_snapshot(0)
                .is_some_and(|s| !s.loading)
        });
        wait_until(|| split.width() >= 1800);
        Self {
            window,
            split,
            content,
            browser,
            preview,
            requests,
            root,
        }
    }

    fn resize(&self, width: i32) {
        self.window.set_width_request(width);
        self.window.set_default_size(width, 800);
        wait_until(|| self.window.width() == width);
    }

    fn settle(&self) {
        let frames = Rc::new(Cell::new(0));
        let completed = frames.clone();
        self.split.add_tick_callback(move |_, _| {
            completed.set(completed.get() + 1);
            if completed.get() >= 2 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        wait_until(|| frames.get() >= 2);
    }

    fn last_column(&self) -> gtk::Widget {
        self.columns().last_child().expect("last column")
    }

    fn enter_children(&self) {
        for depth in 0..2 {
            self.browser.browser().select(depth, 0);
            self.browser.browser().enter_focused_directory();
            wait_until(|| {
                self.browser
                    .browser()
                    .column_snapshot(depth + 1)
                    .is_some_and(|s| !s.loading)
            });
        }
    }

    fn close(self) {
        self.preview.close();
        self.window.destroy();
        self.browser.browser().clear_observer();
    }

    fn columns(&self) -> gtk::Box {
        find(&self.browser.widget(), "columns")
            .expect("columns container")
            .downcast()
            .expect("columns box")
    }

    fn adjustment(&self) -> gtk::Adjustment {
        find(&self.browser.widget(), "columns-scroll")
            .expect("columns viewport")
            .downcast::<gtk::ScrolledWindow>()
            .expect("columns scroller")
            .hadjustment()
    }

    fn wait_adjacent(&self) {
        wait_until(|| {
            let geometry = self.preview.state.geometry(&self.split);
            if self.preview.state.pane.width() != self.preview.widget().width()
                || self.preview.widget().width()
                    != geometry.preview_width(self.preview.state.sizing.manual_width.get())
                || self.split.position()
                    != geometry.position(self.preview.state.sizing.manual_width.get())
            {
                return false;
            }
            let columns = self.columns();
            let Some(last) = columns.last_child() else {
                return false;
            };
            let Some(bounds) = last.compute_bounds(&self.split) else {
                return false;
            };
            let Some(preview) = self.preview.widget().compute_bounds(&self.split) else {
                return false;
            };
            (preview.x() - bounds.x() - bounds.width() - separator_width(&self.split) as f32).abs()
                <= 1.0
        });
    }
}

#[test]
fn browser_and_chooser_keep_the_last_column_visible_as_preview_space_changes() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::browser_and_chooser_keep_the_last_column_visible_as_preview_space_changes",
        || {
            let preferences = ThemeManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            preferences.set_reduce_motion(true);
            for chooser in [false, true] {
                let fixture = Fixture::new(chooser);
                fixture.preview.show(entry("first.png"));
                fixture.wait_adjacent();
                let wide = fixture.preview.widget().width();
                assert!(wide > COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER);
                fixture.content.set_position(260);
                fixture.wait_adjacent();
                assert!(fixture.preview.widget().width() < wide);
                fixture
                    .content
                    .start_child()
                    .expect("sidebar")
                    .set_visible(false);
                fixture.wait_adjacent();
                assert!(fixture.preview.widget().width() > wide);
                fixture
                    .content
                    .start_child()
                    .expect("sidebar")
                    .set_visible(true);
                fixture.content.set_position(180);
                fixture.enter_children();
                fixture.preview.close();
                fixture.resize(1200);
                fixture.preview.show(entry("nested.png"));
                fixture.wait_adjacent();
                wait_until(|| fixture.adjustment().value() > 0.0);
                assert_eq!(
                    fixture.preview.widget().width(),
                    COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER
                );
                let first = fixture.columns().first_child().expect("first column");
                first.set_width_request(COLUMN_WIDTH + 100);
                fixture.wait_adjacent();
                assert_eq!(
                    fixture.preview.widget().width(),
                    COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER
                );
                fixture.resize(1800);
                fixture.wait_adjacent();
                fixture
                    .browser
                    .browser()
                    .navigate(Location::local(fixture.root.path()));
                wait_until(|| {
                    fixture
                        .browser
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|s| !s.loading)
                });
                fixture.wait_adjacent();
                assert_eq!(fixture.adjustment().value(), 0.0);
                fixture.close();
            }
        },
    );
}

#[test]
fn manual_width_overrides_auto_sizing_until_the_window_session_ends() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::manual_width_overrides_auto_sizing_until_the_window_session_ends",
        || {
            let preferences = ThemeManager::shared();
            preferences.set_reduce_motion(true);
            preferences.set_browser_mode(BrowserMode::Columns);
            let fixture = Fixture::new(false);
            fixture.preview.show(entry("first.png"));
            fixture.wait_adjacent();
            fixture
                .preview
                .state
                .resize_preview(&fixture.split, fixture.split.position() + 150);
            let chosen = fixture
                .preview
                .state
                .sizing
                .manual_width
                .get()
                .expect("session width");
            wait_until(|| fixture.preview.widget().width() == chosen);
            fixture.preview.close();
            fixture
                .browser
                .browser()
                .navigate(Location::local(fixture.root.path().join("child")));
            fixture.preview.show(entry("second.png"));
            wait_until(|| fixture.preview.widget().width() == chosen);
            fixture.resize(chosen);
            wait_until(|| fixture.preview.widget().width() < chosen);
            fixture.resize(1800);
            wait_until(|| fixture.preview.widget().width() == chosen);
            for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                preferences.set_browser_mode(mode);
                wait_until(|| fixture.browser.view_mode() == mode);
                wait_until(|| fixture.preview.widget().width() == chosen);
            }
            let second = Fixture::new(false);
            second.preview.show(entry("third.png"));
            second.wait_adjacent();
            assert_ne!(second.preview.widget().width(), chosen);
            for fixture in [fixture, second] {
                fixture.close();
            }
        },
    );
}
