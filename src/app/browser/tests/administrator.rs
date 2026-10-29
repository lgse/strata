// SPDX-License-Identifier: MIT

use super::*;

const ROOT: &str = "/fixture";
const LOCKED: &str = "/fixture/locked";
const ADMIN_LOCKED: &str = "admin:///fixture/locked";

type Activation = fn(&Rc<Browser>);

struct LockedFolderSource {
    offers_administrator: bool,
}

fn folder(path: &str, listable: bool) -> FileEntry {
    let name = Path::new(path).file_name().expect("folder name");
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.to_owned(),
        display_name: name.to_string_lossy().into_owned(),
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Known(listable),
        is_hidden: false,
    }
}

impl FileSource for LockedFolderSource {
    fn offers_administrator_access(&self) -> bool {
        self.offers_administrator
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if location == &Location::local(LOCKED) {
            Err(LocationValidationError::Inaccessible)
        } else {
            Ok(())
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location == Location::local(LOCKED) {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "Permission denied".into(),
                refused: false,
                denied: true,
            });
            return LoadHandle::new(|| {});
        }
        let entries = if request.location == Location::local(ROOT) {
            vec![folder(LOCKED, false), folder("/fixture/open", true)]
        } else {
            Vec::new()
        };
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
        LoadHandle::new(|| {})
    }
}

fn browse(offers_administrator: bool) -> (Rc<Browser>, Rc<RefCell<Vec<BrowserEvent>>>) {
    let browser = Browser::new(Rc::new(LockedFolderSource {
        offers_administrator,
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local(ROOT));
    events.borrow_mut().clear();
    (browser, events)
}

#[test]
fn opening_a_locked_folder_reopens_it_as_administrator() {
    let activations: [(&str, Activation); 3] = [
        ("activate", |browser| browser.activate(0, 0)),
        ("activate in place", |browser| {
            browser.activate_in_place(0, 0)
        }),
        ("activate focused", |browser| {
            browser.select(0, 0);
            browser.activate_focused();
        }),
    ];
    for (route, activate) in activations {
        let (browser, _) = browse(true);
        activate(&browser);
        assert_eq!(
            browser.active_location(),
            Some(Location::uri(ADMIN_LOCKED)),
            "{route}"
        );

        let (browser, events) = browse(false);
        activate(&browser);
        let events = events.borrow();
        let rejected = events.iter().any(|event| {
            matches!(
                event,
                BrowserEvent::NavigationRejected {
                    location,
                    error: LocationValidationError::Inaccessible,
                    ..
                } if location == &Location::local(LOCKED)
            )
        });
        let failed = events.iter().any(|event| {
            matches!(
                event,
                BrowserEvent::LoadFailed {
                    administrator: None,
                    ..
                }
            )
        });
        assert!(
            rejected || failed,
            "{route} should report the permission error"
        );
        assert!(
            !browser
                .active_location()
                .is_some_and(|location| location.is_administrator()),
            "{route}"
        );
    }
}

#[test]
fn moving_onto_a_locked_folder_offers_administrator_access_without_prompting() {
    for offers_administrator in [true, false] {
        let (browser, events) = browse(offers_administrator);
        let expected = offers_administrator.then(|| Location::uri(ADMIN_LOCKED));

        browser.select(0, 0);
        browser.show_child(0, Location::local(LOCKED));

        assert_eq!(browser.location_at(1), Some(Location::local(LOCKED)));
        assert!(
            events.borrow().iter().all(|event| !matches!(
                event,
                BrowserEvent::NavigationRejected { .. }
                    | BrowserEvent::LocationNavigationRejected { .. }
            )),
            "moving the selection must not validate, so it never opens the error dialog"
        );
        assert!(events.borrow().iter().any(|event| matches!(
            event,
            BrowserEvent::LoadFailed { depth: 1, administrator, .. } if administrator == &expected
        )));
        assert_eq!(
            browser
                .column_snapshot(1)
                .and_then(|snapshot| snapshot.administrator),
            expected
        );

        browser.activate(0, 0);
        assert_eq!(
            browser.active_location() == Some(Location::uri(ADMIN_LOCKED)),
            offers_administrator,
            "activating the already shown folder still reopens it as administrator"
        );
    }
}
