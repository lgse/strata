// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::services::{
    DirectoryEvent, DirectoryRequest, LoadHandle, LocationValidationError, RequestId,
};

struct Request {
    id: RequestId,
    emit: Rc<dyn Fn(DirectoryEvent)>,
}

#[derive(Default)]
struct HeldSource(RefCell<Option<Request>>);

impl FileSource for HeldSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        *self.0.borrow_mut() = Some(Request {
            id: request.id,
            emit,
        });
        LoadHandle::new(|| {})
    }
}

impl HeldSource {
    fn finish(&self) {
        let request = self.0.borrow();
        let request = request.as_ref().expect("directory request");
        (request.emit)(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
    }

    fn fail(&self) {
        let request = self.0.borrow();
        let request = request.as_ref().expect("directory request");
        (request.emit)(DirectoryEvent::Failed {
            request_id: request.id,
            message: "Synthetic load failure".into(),
        });
    }

    fn batch(&self, root: &std::path::Path) {
        let request = self.0.borrow();
        let request = request.as_ref().expect("directory request");
        (request.emit)(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![FileEntry {
                location: Location::local(root.join("example.txt")),
                thumbnail_path: None,
                native_name: "example.txt".into(),
                display_name: "example.txt".into(),
                kind: crate::model::EntryKind::File,
                size: crate::model::MetadataValue::Unknown,
                modified_unix_seconds: crate::model::MetadataValue::Unknown,
                mode: crate::model::MetadataValue::Unknown,
                is_hidden: false,
            }],
        });
    }
}

fn stacks(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Stack> {
    let mut found = Vec::new();
    if let Some(stack) = widget.as_ref().downcast_ref::<gtk::Stack>()
        && stack.child_by_name("loading").is_some()
    {
        found.push(stack.clone());
    }
    let mut child = widget.as_ref().first_child();
    while let Some(widget) = child {
        found.extend(stacks(&widget));
        child = widget.next_sibling();
    }
    found
}

fn assert_page(stacks: &[gtk::Stack], page: &str) {
    assert!(!stacks.is_empty());
    for stack in stacks {
        assert_eq!(stack.visible_child_name().as_deref(), Some(page));
    }
}

fn settle() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
#[ignore = "captures loading UI; requires STRATA_LOADING_CAPTURE and a private display"]
fn capture_loading_frame() {
    gtk::init().expect("private GTK display");
    crate::assets::prepare().expect("bundled assets");
    crate::assets::register_icon_theme();
    let output = std::env::var_os("STRATA_LOADING_CAPTURE").expect("capture path");
    crate::ui::prepare_portal_ui();
    let source = Rc::new(HeldSource::default());
    let view = BrowserView::new(source, PeekBehavior::default());
    view.set_view_mode(BrowserMode::List);
    let window = gtk::Window::builder()
        .title("Directory loading — first frame")
        .default_width(850)
        .default_height(450)
        .child(&view.widget())
        .build();
    window.present();
    settle();
    view.browser()
        .navigate(Location::local("/tmp/strata-loading-example"));
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(40) {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    let node = snapshot.to_node().expect("rendered browser");
    window
        .renderer()
        .expect("renderer")
        .render_texture(&node, None)
        .save_to_png(std::path::PathBuf::from(output))
        .expect("save capture");
    window.close();
    view.browser().clear_observer();
}

#[test]
fn directory_loading_grace_across_modes() {
    crate::test_support::gtk_test(
        "ui::browser::tests::loading::directory_loading_grace_across_modes",
        || {
            let root = tempfile::tempdir().expect("fixture directory");
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                view.set_view_mode(mode);
                let browser = view.browser();
                browser.navigate(Location::local(root.path()));
                let initial = stacks(&view.widget());
                assert_page(&initial, "pending");
                let flashed = Rc::new(Cell::new(false));
                for stack in &initial {
                    let flashed = flashed.clone();
                    stack.connect_visible_child_name_notify(move |stack| {
                        if stack.visible_child_name().as_deref() == Some("loading") {
                            flashed.set(true);
                        }
                    });
                }
                source.batch(root.path());
                source.finish();
                settle();
                assert_page(&initial, "content");
                assert!(!flashed.get(), "fast loads must never display the skeleton");

                browser.navigate(Location::local(root.path().join("slow")));
                let slow = stacks(&view.widget());
                assert_page(&slow, "pending");
                settle();
                assert_page(&slow, "loading");
                source.batch(root.path());
                source.finish();
                settle();
                assert_page(&slow, "content");
                browser.reload_active();
                let reload = stacks(&view.widget());
                assert_page(&reload, "pending");
                source.batch(root.path());
                source.finish();
                settle();
                assert_page(&reload, "content");

                for fail in [false, true] {
                    browser.navigate(Location::local(root.path().join(if fail {
                        "error"
                    } else {
                        "empty"
                    })));
                    let pending = stacks(&view.widget());
                    assert_page(&pending, "pending");
                    if fail {
                        source.fail();
                    } else {
                        source.finish();
                    }
                    settle();
                    for stack in pending {
                        let expected = if stack.child_by_name("feedback").is_some() {
                            "feedback"
                        } else {
                            "status"
                        };
                        assert_eq!(stack.visible_child_name().as_deref(), Some(expected));
                    }
                }

                browser.navigate(Location::local(root.path().join("abandoned")));
                let abandoned = stacks(&view.widget());
                assert_page(&abandoned, "pending");
                browser.navigate(Location::local(root.path().join("replacement")));
                source.batch(root.path());
                source.finish();
                settle();
                assert_page(&abandoned, "pending");
                assert_page(&stacks(&view.widget()), "content");

                browser.clear_observer();
            }
        },
    );
}
