// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{
    DirectoryEvent, DirectoryRequest, LoadHandle, LocationValidationError, RequestId,
};

mod camera_stress;

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
        self.batch_at(Location::local(root.join("example.txt")));
    }

    fn batch_at(&self, location: Location) {
        let name = location.display_name();
        let request = self.0.borrow();
        let request = request.as_ref().expect("directory request");
        (request.emit)(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![FileEntry {
                location,
                thumbnail_path: None,
                native_name: name.clone().into(),
                display_name: name,
                kind: crate::model::EntryKind::File,
                size: crate::model::MetadataValue::Unknown,
                modified_unix_seconds: crate::model::MetadataValue::Unknown,
                mode: crate::model::MetadataValue::Unknown,
                recent_unix_seconds: crate::model::MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: crate::model::MetadataValue::Unknown,
                child_count: crate::model::MetadataValue::Unknown,
                duration_seconds: crate::model::MetadataValue::Unknown,
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
fn parked_keyboard_navigation_selects_the_requested_endpoint() {
    crate::test_support::gtk_test(
        "ui::browser::tests::loading::parked_keyboard_navigation_selects_the_requested_endpoint",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                view.set_view_mode(mode);
                let outside = gtk::Entry::new();
                let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
                root.append(&outside);
                root.append(&view.widget());
                let window = gtk::Window::builder()
                    .child(&root)
                    .default_width(900)
                    .default_height(600)
                    .build();
                window.present();
                view.browser().navigate(Location::local("/fixture"));
                source.batch_at(Location::local("/fixture/a.txt"));
                source.batch_at(Location::local("/fixture/z.txt"));
                source.finish();
                settle();
                let stack = stacks(&view.widget())
                    .into_iter()
                    .find(|stack| stack.is_mapped())
                    .expect("visible pane");
                for (direction, expected) in [(1, "z.txt"), (-1, "a.txt")] {
                    assert!(stack.grab_focus());
                    assert!(view.jump_parked_selection(direction));
                    settle();
                    assert_eq!(
                        view.browser()
                            .focused_entry()
                            .expect("selected endpoint")
                            .display_name,
                        expected
                    );
                    assert!(outside.grab_focus());
                    assert!(!view.jump_parked_selection(-direction));
                    assert_eq!(
                        view.browser()
                            .focused_entry()
                            .expect("preserved endpoint")
                            .display_name,
                        expected
                    );
                }
                window.close();
            }
        },
    );
}

#[test]
fn camera_first_batch_is_visible_before_discovery_finishes_in_every_view() {
    crate::test_support::gtk_test(
        "ui::browser::tests::loading::camera_first_batch_is_visible_before_discovery_finishes_in_every_view",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                view.set_view_mode(mode);
                crate::ui::thumbnail::hold_thumbnail_workers();
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(900)
                    .default_height(600)
                    .build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::uri("gphoto2://camera/"));
                let current_stacks = || {
                    stacks(
                        &view
                            .state
                            .mode_views
                            .borrow()
                            .widget()
                            .visible_child()
                            .expect("visible browser mode"),
                    )
                };
                for reload in [false, true] {
                    if reload {
                        browser.reload_active();
                    }
                    settle();
                    let panes = current_stacks();
                    assert_page(&panes, "loading");
                    source.batch_at(Location::uri("gphoto2://camera/202606/IMG_0001.JPG"));
                    settle();
                    assert_page(&panes, "content");
                    assert!(browser.column_snapshot(0).expect("camera root").loading);
                    crate::ui::thumbnail::tests::complete_pending_thumbnail(std::path::Path::new(
                        "gphoto2://camera/202606/IMG_0001.JPG",
                    ));
                    assert!(browser.column_snapshot(0).expect("camera root").loading);
                    let alternate = match mode {
                        BrowserMode::Columns => BrowserMode::List,
                        BrowserMode::List => BrowserMode::Icons,
                        BrowserMode::Icons => BrowserMode::Columns,
                    };
                    view.set_view_mode(alternate);
                    settle();
                    assert_page(&current_stacks(), "content");
                    view.set_view_mode(mode);
                    settle();
                    assert_page(&current_stacks(), "content");
                    source.batch_at(Location::uri("gphoto2://camera/202605/IMG_0001.JPG"));
                    settle();
                    assert_page(&current_stacks(), "content");
                    assert_eq!(browser.column_snapshot(0).expect("camera root").count, 2);
                    source.finish();
                    settle();
                    assert_page(&current_stacks(), "content");
                }
                browser.clear_observer();
                crate::ui::thumbnail::cancel_thumbnails_in(&view.widget());
                window.destroy();
                crate::ui::thumbnail::clear_thumbnail_runtime();
            }
        },
    );
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
                for stack in &slow {
                    let loading = stack.visible_child().expect("loading placeholder");
                    assert!(
                        !loading.can_target(),
                        "placeholder must not intercept input"
                    );
                    assert!(!loading.is_focusable(), "placeholder must not take focus");
                }
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
