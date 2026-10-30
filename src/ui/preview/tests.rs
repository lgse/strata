// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{Location, MetadataValue};

mod audio;
mod find;
mod media_controls;
mod model;
mod operations;
mod pdf;
mod route;
mod video;

struct Pending {
    request: PreviewRequest,
    emit: Rc<dyn Fn(PreviewEvent)>,
}

#[derive(Default)]
struct Provider(RefCell<Vec<Pending>>);

impl PreviewProvider for Provider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.0.borrow_mut().push(Pending { request, emit });
        LoadHandle::new(|| {})
    }
}

pub(super) fn entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(name),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

#[test]
fn comic_and_epub_selections_are_quick_preview_targets() {
    for name in ["sample.cbz", "sample.cbr", "sample.epub"] {
        assert!(preview_target(Some(entry(name))).is_some(), "{name}");
    }
}

#[test]
fn ambiguous_names_are_quick_preview_targets_for_the_loader_to_resolve() {
    for name in ["app.ts", "module.mts", "notes.zig"] {
        assert!(preview_target(Some(entry(name))).is_some(), "{name}");
    }
}

#[test]
fn model_progress_and_theme_reloads_follow_the_current_request_in_each_drawer() {
    crate::test_support::gtk_test(
        "ui::preview::tests::model_progress_and_theme_reloads_follow_the_current_request_in_each_drawer",
        || {
            let provider = Rc::new(Provider::default());
            let first = PreviewDrawer::new(provider.clone(), false);
            let second = PreviewDrawer::new(provider.clone(), false);
            let manager = super::super::theme::ThemeManager::shared();
            first.show(entry("old.stl"), None);
            second.show(entry("second.stl"), None);
            first.show(entry("new.stl"), None);
            let emit_progress = |index: usize, stage| {
                let pending = provider.0.borrow();
                let pending = &pending[index];
                (pending.emit)(PreviewEvent::Progress {
                    request_id: pending.request.id,
                    stage,
                });
            };
            emit_progress(
                2,
                crate::services::ModelPreviewStage::Rendering { triangles: 23 },
            );
            let triangles = first
                .state
                .model
                .borrow()
                .as_ref()
                .expect("loading model UI")
                .triangles
                .clone();
            assert_eq!(triangles.text(), "23 triangles");
            emit_progress(
                0,
                crate::services::ModelPreviewStage::Rendering { triangles: 99 },
            );
            assert_eq!(triangles.text(), "23 triangles");
            assert!(first.state.model_spinner.is_visible());
            {
                let pending = provider.0.borrow();
                (pending[0].emit)(PreviewEvent::Failed {
                    request_id: pending[0].request.id,
                    entry: entry("old.stl"),
                    message: "stale failure".into(),
                });
            }
            assert!(first.state.model.borrow().is_some(), "current model");
            assert!(first.state.model_spinner.is_visible());
            model::ready(&provider, 2, Some(crate::services::ModelView::FrontLeft));
            model::ready(&provider, 1, Some(crate::services::ModelView::FrontLeft));
            let next = first
                .state
                .model
                .borrow()
                .as_ref()
                .expect("mesh controls")
                .next
                .clone();
            next.emit_clicked();
            model::ready(&provider, 3, Some(crate::services::ModelView::BackLeft));
            let old_palette = manager.active_model_palette();
            let mut tokens = manager.appearance_tokens();
            tokens.accent = if old_palette.accent == 0xff0000 {
                "#00ff00"
            } else {
                "#ff0000"
            }
            .into();
            manager.preview(&tokens);
            {
                let pending = provider.0.borrow();
                assert_eq!(pending.len(), 6);
                assert_eq!(pending[4].request.entry.native_name, "new.stl");
                assert_eq!(
                    pending[4].request.model_view,
                    crate::services::ModelView::BackLeft
                );
                assert_eq!(pending[5].request.entry.native_name, "second.stl");
                assert_eq!(
                    pending[5].request.model_view,
                    crate::services::ModelView::FrontLeft
                );
                for request in &pending[4..] {
                    assert_eq!(
                        request.request.model_palette,
                        manager.active_model_palette()
                    );
                    assert_ne!(request.request.model_palette, old_palette);
                }
            }
            first.close();
            emit_progress(4, crate::services::ModelPreviewStage::Finishing);
            assert!(first.state.current_request.get().is_none());
            assert!(first.state.model.borrow().is_none());
            tokens.surface = "#102030".into();
            manager.preview(&tokens);
            assert_eq!(
                provider.0.borrow().len(),
                7,
                "closed drawer must not reload"
            );
            second.close();
        },
    );
}

struct EmptySource;

impl crate::services::FileSource for EmptySource {
    fn validate_location(
        &self,
        _location: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> LoadHandle {
        emit(crate::services::DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn an_explicit_close_blocks_automatic_previews_until_reopened() {
    crate::test_support::gtk_test(
        "ui::preview::tests::an_explicit_close_blocks_automatic_previews_until_reopened",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let browser = Browser::new(Rc::new(EmptySource));
            let automatic = BrowserEvent::PreviewRequested {
                entry: entry("mirrored.txt"),
                automatic: true,
            };

            drawer.handle_browser_event(&browser, &automatic);
            assert!(
                drawer.is_enabled(),
                "a drawer never closed follows the first mirror"
            );
            assert_eq!(provider.0.borrow().len(), 1);

            drawer.close();
            drawer.handle_browser_event(&browser, &automatic);
            assert!(
                !drawer.is_enabled(),
                "mirroring must not reopen a dismissed drawer"
            );
            assert_eq!(provider.0.borrow().len(), 1);

            drawer.handle_browser_event(
                &browser,
                &BrowserEvent::PreviewRequested {
                    entry: entry("clicked.txt"),
                    automatic: false,
                },
            );
            assert!(
                drawer.is_enabled(),
                "an explicit request reopens the drawer"
            );
            drawer.handle_browser_event(&browser, &automatic);
            assert_eq!(
                provider.0.borrow().len(),
                3,
                "mirroring follows again after an explicit reopen"
            );
            drawer.close();

            crate::ui::preferences::PreferenceManager::shared()
                .set_browser_mode(crate::ui::browser_modes::BrowserMode::Columns);
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(EmptySource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let released = PreviewDrawer::new(provider.clone(), false);
            let split = gtk::Paned::new(gtk::Orientation::Horizontal);
            let content = gtk::Paned::new(gtk::Orientation::Horizontal);
            released.attach_split(&split, &content, &view, None);
            assert!(!released.is_enabled());
            assert!(released.state.reserves_column_space());
            released.state.toggle_panel(None, None);
            released.handle_browser_event(&browser, &automatic);
            assert!(
                !released.is_enabled(),
                "releasing the panel before any preview also blocks mirroring"
            );
            assert_eq!(provider.0.borrow().len(), 3);
        },
    );
}

#[test]
fn preview_loads_on_first_show_when_sidebar_rails_in_narrow_split() {
    crate::test_support::gtk_test(
        "ui::preview::tests::preview_loads_on_first_show_when_sidebar_rails_in_narrow_split",
        || {
            let provider = Rc::new(Provider::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_browser_mode(crate::ui::browser_modes::BrowserMode::List);
            let browser = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let sidebar =
                crate::ui::window::build_sidebar(browser.clone(), preferences.clone(), false);

            let window = gtk::Window::builder()
                .default_width(700)
                .default_height(500)
                .build();
            let split = gtk::Paned::new(gtk::Orientation::Horizontal);
            let content = gtk::Paned::new(gtk::Orientation::Horizontal);
            content.set_start_child(Some(&sidebar.widget));
            content.set_end_child(Some(&browser.widget()));
            content.set_position(crate::ui::window::preferred_sidebar_width());
            split.set_start_child(Some(&content));
            split.set_end_child(Some(&preview.widget()));
            window.set_child(Some(&split));

            preview.attach_split(&split, &content, &browser, Some(&sidebar));
            window.present();

            while glib::MainContext::default().iteration(false) {}

            assert!(!sidebar.state.rail.get(), "sidebar should start unrailed");
            assert!(!preview.is_open(), "preview should start closed");

            preview.show(entry("sample.txt"), None);

            while glib::MainContext::default().iteration(false) {}

            assert!(preview.is_open(), "preview drawer should be open");
            assert!(
                sidebar.state.rail.get(),
                "sidebar should be railed to fit preview"
            );
            assert_eq!(
                preview.state.title.text(),
                "sample.txt",
                "preview header title must be populated on first view in narrow window"
            );
            assert_eq!(
                provider.0.borrow().len(),
                1,
                "preview content must be loaded on first view in narrow window"
            );
            assert_eq!(
                provider.0.borrow()[0].request.entry.display_name,
                "sample.txt"
            );

            window.destroy();
        },
    );
}

#[test]
fn keyboard_divider_moves_lower_the_columns_session_minimum_without_moving_the_panel() {
    crate::test_support::gtk_test(
        "ui::preview::tests::keyboard_divider_moves_lower_the_columns_session_minimum_without_moving_the_panel",
        || {
            let provider = Rc::new(Provider::default());
            let preview = PreviewDrawer::new(provider, false);
            crate::ui::preferences::PreferenceManager::shared()
                .set_browser_mode(crate::ui::browser_modes::BrowserMode::Columns);
            let root = tempfile::tempdir().expect("preview fixture");
            std::fs::write(root.path().join("sample.txt"), b"sample").expect("fixture file");
            let browser = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let window = gtk::Window::builder()
                .default_width(1200)
                .default_height(700)
                .build();
            let split = gtk::Paned::new(gtk::Orientation::Horizontal);
            let content = gtk::Paned::new(gtk::Orientation::Horizontal);
            content.set_end_child(Some(&browser.widget()));
            split.set_start_child(Some(&content));
            split.set_end_child(Some(&preview.widget()));
            window.set_child(Some(&split));
            preview.attach_split(&split, &content, &browser, None);
            window.present();
            browser.browser().navigate(Location::local(root.path()));
            preview.show(entry("sample.txt"), None);
            let settle = || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
                while std::time::Instant::now() < deadline {
                    while glib::MainContext::default().iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            settle();
            assert!(preview.is_open());
            let filled = split.position();
            let shown = preview.state.pane.width();
            assert!(preview.state.sizing.manual_width_for_test().is_none());

            split.grab_focus();
            assert!(split.has_focus(), "the divider takes keyboard focus");
            split.emit_by_name::<bool>("move-handle", &[&gtk::ScrollType::StepRight]);
            settle();

            assert_eq!(
                split.position(),
                filled,
                "the preview still fills the free space"
            );
            let minimum = preview
                .state
                .sizing
                .manual_width_for_test()
                .expect("a keyboard move sets the session minimum");
            assert!(
                minimum < shown,
                "narrowing lowers the minimum below the filled width ({minimum} vs {shown})"
            );
            window.destroy();
        },
    );
}

#[test]
fn only_a_docked_preview_offers_its_divider_for_resizing() {
    crate::test_support::gtk_test(
        "ui::preview::tests::only_a_docked_preview_offers_its_divider_for_resizing",
        || {
            let provider = Rc::new(Provider::default());
            let preview = PreviewDrawer::new(provider, false);
            crate::ui::preferences::PreferenceManager::shared()
                .set_browser_mode(crate::ui::browser_modes::BrowserMode::Columns);
            let browser = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let window = gtk::Window::builder()
                .default_width(1200)
                .default_height(700)
                .build();
            let split = gtk::Paned::new(gtk::Orientation::Horizontal);
            split.set_wide_handle(true);
            let content = gtk::Paned::new(gtk::Orientation::Horizontal);
            content.set_end_child(Some(&browser.widget()));
            split.set_start_child(Some(&content));
            split.set_end_child(Some(&preview.widget()));
            window.set_child(Some(&split));
            preview.attach_split(&split, &content, &browser, None);
            window.present();
            let settle = || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
                while std::time::Instant::now() < deadline {
                    while glib::MainContext::default().iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            let divider = super::separator(&split).expect("preview divider");
            let offered = || preview.state.resize_grip.is_visible() && divider.cursor().is_some();
            settle();
            assert!(!offered(), "a hidden preview offers no resize");

            preview.show(entry("sample.txt"), None);
            settle();
            assert!(offered(), "a docked preview offers its grip and divider");

            preview.state.toggle_panel(None, None);
            settle();
            assert!(!offered(), "closing the preview withdraws its resize");
            window.destroy();
        },
    );
}
