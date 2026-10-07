// SPDX-License-Identifier: MIT

use std::{cell::RefCell, collections::HashMap, ffi::OsStr};

use gtk::prelude::*;

use super::*;
use crate::{
    model::{EntryKind, FileEntry, MetadataValue},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError,
    },
    test_support::gtk_test,
    ui::browser::PeekBehavior,
};

type Validation = Rc<dyn Fn(Result<(), LocationValidationError>)>;
type Watch = Rc<dyn Fn(DirectoryChange)>;

#[derive(Default)]
struct HeldSource {
    validation: RefCell<Option<Validation>>,
    immediate: RefCell<Option<Result<(), LocationValidationError>>>,
    blocked: RefCell<Option<Location>>,
    cancelled: Rc<RefCell<Vec<Location>>>,
    enumerated: RefCell<Vec<Location>>,
    watchers: RefCell<HashMap<Location, Watch>>,
}

impl FileSource for HeldSource {
    fn allows_navigation(&self, location: &Location) -> bool {
        self.blocked.borrow().as_ref() != Some(location)
    }

    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn validate_location_async(&self, location: Location, emit: Validation) -> LoadHandle {
        let immediate = self.immediate.take();
        if let Some(result) = immediate {
            emit(result);
        } else {
            self.validation.replace(Some(emit));
        }
        let cancelled = self.cancelled.clone();
        LoadHandle::new(move || cancelled.borrow_mut().push(location))
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.enumerated.borrow_mut().push(request.location);
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }

    fn watch(&self, location: Location, _: bool, notify: Watch) -> Option<LoadHandle> {
        self.watchers.borrow_mut().insert(location, notify);
        Some(LoadHandle::new(|| {}))
    }
}

fn renamed_directory(location: Location) -> FileEntry {
    let name = location.display_name();
    FileEntry {
        location,
        thumbnail_path: None,
        native_name: name.clone().into(),
        display_name: name,
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn remote_tab_titles_wait_for_validation_and_release_on_rejection_or_superseding_navigation() {
    gtk_test(
        "ui::browser::tab_location::tests::remote_tab_titles_wait_for_validation_and_release_on_rejection_or_superseding_navigation",
        || {
            for outcome in [
                "success",
                "rejected",
                "superseded",
                "same_location",
                "sync_success",
                "sync_rejected",
            ] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                let window = gtk::Window::builder().child(&view.widget()).build();
                window.present();
                let parent = Location::uri("sftp://fixture/parent");
                let alpha = Location::uri("sftp://fixture/parent/alpha");
                let beta = Location::uri("sftp://fixture/parent/beta");
                let browser = view.browser();
                browser.navigate(parent.clone());
                browser.descend(0, alpha.clone());
                let complete = source.validation.take().expect("first validation");
                complete(Ok(()));
                let changes = Rc::new(RefCell::new(Vec::new()));
                let observed = changes.clone();
                view.observe_tab_location(move |location| {
                    observed.borrow_mut().push(location.cloned());
                });
                changes.borrow_mut().clear();
                let hold = view.state.hold_tab_location();
                browser.set_active_column(0);
                browser.focus_active();
                if outcome.starts_with("sync_") {
                    source.immediate.replace(Some(if outcome == "sync_success" {
                        Ok(())
                    } else {
                        Err(LocationValidationError::Inaccessible)
                    }));
                }
                hold.navigate(0, || browser.descend(0, beta.clone()));
                if outcome.starts_with("sync_") {
                    let expected = if outcome == "sync_success" {
                        beta
                    } else {
                        parent
                    };
                    assert_eq!(browser.pending_navigation_generation(), None);
                    assert_eq!(browser.active_location(), Some(expected.clone()));
                    assert_eq!(&*changes.borrow(), &[Some(expected)]);
                    browser.clear_observer();
                    window.destroy();
                    continue;
                }
                assert!(
                    changes.borrow().is_empty(),
                    "{outcome}: awaiting validation"
                );
                assert_eq!(browser.active_location(), Some(parent.clone()));
                let complete = source.validation.take().expect("held validation");
                let expected = match outcome {
                    "success" => {
                        complete(Ok(()));
                        beta
                    }
                    "rejected" => {
                        complete(Err(LocationValidationError::Inaccessible));
                        parent
                    }
                    "superseded" => {
                        let replacement = Location::uri("sftp://fixture/replacement");
                        browser.navigate(replacement.clone());
                        complete(Ok(()));
                        replacement
                    }
                    "same_location" => {
                        let enumerations = source.enumerated.borrow().len();
                        browser.navigate(parent.clone());
                        assert_eq!(&*changes.borrow(), &[Some(parent.clone())]);
                        assert!(source.cancelled.borrow().contains(&beta));
                        assert_eq!(source.enumerated.borrow().len(), enumerations);
                        complete(Ok(()));
                        parent
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    &*changes.borrow(),
                    &[Some(expected.clone())],
                    "{outcome}: final title"
                );
                assert_eq!(browser.active_location(), Some(expected));
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn boundary_refused_descent_releases_the_title_without_navigation_events() {
    gtk_test(
        "ui::browser::tab_location::tests::boundary_refused_descent_releases_the_title_without_navigation_events",
        || {
            for parent in [
                Location::local("/fixture"),
                Location::uri("sftp://fixture/parent"),
            ] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                let window = gtk::Window::builder().child(&view.widget()).build();
                window.present();
                let browser = view.browser();
                let alpha = parent.child(OsStr::new("alpha")).expect("first child");
                let blocked = parent.child(OsStr::new("blocked")).expect("blocked child");
                browser.navigate(parent.clone());
                browser.descend(0, alpha.clone());
                if let Some(complete) = source.validation.take() {
                    complete(Ok(()));
                }
                source.blocked.replace(Some(blocked.clone()));
                let changes = Rc::new(RefCell::new(Vec::new()));
                let observed = changes.clone();
                view.observe_tab_location(move |location| {
                    observed.borrow_mut().push(location.cloned());
                });
                changes.borrow_mut().clear();
                let hold = view.state.hold_tab_location();
                browser.set_active_column(0);
                browser.focus_active();
                hold.navigate(0, || browser.descend(0, blocked.clone()));
                if let Some(complete) = source.validation.take() {
                    assert!(changes.borrow().is_empty());
                    complete(Ok(()));
                }
                assert_eq!(&*changes.borrow(), &[Some(parent.clone())]);
                assert_eq!(browser.active_location(), Some(parent));
                assert_eq!(browser.location_at(1), Some(alpha));
                assert!(!source.enumerated.borrow().contains(&blocked));
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn relocation_settles_only_the_navigation_context_it_invalidates() {
    gtk_test(
        "ui::browser::tab_location::tests::relocation_settles_only_the_navigation_context_it_invalidates",
        || {
            for parent_moved in [false, true] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                let window = gtk::Window::builder().child(&view.widget()).build();
                window.present();
                let browser = view.browser();
                let root = Location::uri("sftp://fixture/root");
                let parent = Location::uri("sftp://fixture/root/parent");
                let alpha = Location::uri("sftp://fixture/root/parent/alpha");
                let beta = Location::uri("sftp://fixture/root/parent/beta");
                browser.navigate(root.clone());
                browser.descend(0, parent.clone());
                source.validation.take().expect("parent validation")(Ok(()));
                browser.descend(1, alpha.clone());
                source.validation.take().expect("child validation")(Ok(()));
                let changes = Rc::new(RefCell::new(Vec::new()));
                let observed = changes.clone();
                view.observe_tab_location(move |location| {
                    observed.borrow_mut().push(location.cloned());
                });
                changes.borrow_mut().clear();
                let hold = view.state.hold_tab_location();
                browser.set_active_column(1);
                browser.focus_active();
                if parent_moved {
                    hold.navigate(1, || browser.descend(1, beta.clone()));
                    let complete = source.validation.take().expect("pending child validation");
                    let renamed = Location::uri("sftp://fixture/root/renamed");
                    let notify = source
                        .watchers
                        .borrow()
                        .get(&root)
                        .expect("root watcher")
                        .clone();
                    notify(DirectoryChange::Move {
                        from: parent,
                        entry: renamed_directory(renamed.clone()),
                    });
                    assert_eq!(&*changes.borrow(), &[Some(renamed.clone())]);
                    complete(Ok(()));
                    assert_eq!(browser.active_location(), Some(renamed.clone()));
                    assert_eq!(&*changes.borrow(), &[Some(renamed)]);
                } else {
                    let notify = source
                        .watchers
                        .borrow()
                        .get(&parent)
                        .expect("parent watcher")
                        .clone();
                    notify(DirectoryChange::Move {
                        from: alpha,
                        entry: renamed_directory(Location::uri("sftp://fixture/root/parent/omega")),
                    });
                    assert!(changes.borrow().is_empty());
                    hold.navigate(1, || browser.descend(1, beta.clone()));
                    source.validation.take().expect("sibling validation")(Ok(()));
                    assert_eq!(browser.active_location(), Some(beta.clone()));
                    assert_eq!(&*changes.borrow(), &[Some(beta)]);
                }
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn older_validation_completion_preserves_a_newer_pressed_hold() {
    gtk_test(
        "ui::browser::tab_location::tests::older_validation_completion_preserves_a_newer_pressed_hold",
        || {
            for rejected in [false, true] {
                let source = Rc::new(HeldSource::default());
                let view = BrowserView::new(source.clone(), PeekBehavior::default());
                let window = gtk::Window::builder().child(&view.widget()).build();
                window.present();
                let browser = view.browser();
                let parent = Location::uri("sftp://fixture/parent");
                let alpha = Location::uri("sftp://fixture/parent/alpha");
                let beta = Location::uri("sftp://fixture/parent/beta");
                let gamma = Location::uri("sftp://fixture/parent/gamma");
                browser.navigate(parent.clone());
                browser.descend(0, alpha);
                let complete = source.validation.take().expect("initial validation");
                complete(Ok(()));
                let changes = Rc::new(RefCell::new(Vec::new()));
                let observed = changes.clone();
                view.observe_tab_location(move |location| {
                    observed.borrow_mut().push(location.cloned());
                });
                changes.borrow_mut().clear();
                let older = view.state.hold_tab_location();
                browser.set_active_column(0);
                browser.focus_active();
                older.navigate(0, || browser.descend(0, beta));
                let complete = source.validation.take().expect("older validation");
                let newer = view.state.hold_tab_location();
                complete(if rejected {
                    Err(LocationValidationError::Inaccessible)
                } else {
                    Ok(())
                });
                assert!(changes.borrow().is_empty());
                newer.navigate(0, || browser.descend(0, gamma.clone()));
                let complete = source.validation.take().expect("newer click must navigate");
                assert!(changes.borrow().is_empty());
                complete(Ok(()));
                assert_eq!(browser.active_location(), Some(gamma.clone()));
                assert_eq!(&*changes.borrow(), &[Some(gamma)]);
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}
