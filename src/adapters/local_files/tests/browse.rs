// SPDX-License-Identifier: MIT

use super::*;
use crate::app::{Browser, BrowserEvent};

fn browse_info(file_type: gio::FileType, target: Option<&str>, attributes: &str) -> gio::FileInfo {
    let info = gio::FileInfo::new();
    info.set_name("share-alias");
    info.set_display_name("Documents");
    info.set_file_type(file_type);
    if let Some(target) = target {
        info.set_attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI, target);
    }
    let matcher = gio::FileAttributeMatcher::new(attributes);
    for attribute in info.list_attributes(None) {
        if !matcher.matches(&attribute) {
            info.remove_attribute(&attribute);
        }
    }
    info
}

#[test]
fn browse_entries_resolve_destinations_in_streaming_and_metadata_listings() {
    for attributes in [LIST_ATTRIBUTES, FULL_ATTRIBUTES] {
        for file_type in [gio::FileType::Shortcut, gio::FileType::Mountable] {
            for target in [
                "smb://server/Documents",
                "smb://server/Shared%20Documents",
                "smb://server/",
            ] {
                let expected = location_for_file(&gio::File::for_uri(target))
                    .expect("the browse target should be a valid location");
                let entry = entry_from_info(
                    Location::uri("network:///share-alias"),
                    browse_info(file_type, Some(target), attributes),
                );
                assert_eq!(entry.location, expected, "{file_type:?}, {attributes}");
                assert!(entry.is_directory());
                assert_eq!(entry.native_name, "share-alias");
                assert_eq!(entry.display_name, "Documents");
                assert_eq!(entry.size, MetadataValue::Unknown);
            }
        }
    }
}

#[test]
fn browse_entries_without_usable_targets_keep_the_original_location() {
    let location = Location::uri("smb://server/share-alias");
    for file_type in [gio::FileType::Shortcut, gio::FileType::Mountable] {
        for target in [None, Some(""), Some("not-a-uri")] {
            let entry = entry_from_info(
                location.clone(),
                browse_info(file_type, target, LIST_ATTRIBUTES),
            );
            assert_eq!(entry.location, location);
            assert!(entry.is_directory());
        }
    }
}

#[test]
fn ordinary_entries_do_not_redirect_to_target_uris() {
    for file_type in [
        gio::FileType::Regular,
        gio::FileType::Directory,
        gio::FileType::SymbolicLink,
    ] {
        for location in [
            Location::uri("trash:///share-alias"),
            Location::uri("smb://server/share/share-alias"),
        ] {
            let entry = entry_from_info(
                location.clone(),
                browse_info(file_type, Some("file:///fixture/target"), FULL_ATTRIBUTES),
            );
            assert_eq!(entry.location, location);
        }
    }
}

struct BrowseSource {
    entry: FileEntry,
    validation_error: Option<LocationValidationError>,
}

impl FileSource for BrowseSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        assert_eq!(location, &Location::uri("smb://server/Documents"));
        self.validation_error.clone().map_or(Ok(()), Err)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location == Location::uri("smb://server/") {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![self.entry.clone()],
            });
        }
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: Some(false),
            can_delete: Some(false),
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn browse_activation_navigates_or_requests_mounting_of_the_resolved_share() {
    let root = Location::uri("smb://server/");
    let target = Location::uri("smb://server/Documents");
    for file_type in [gio::FileType::Shortcut, gio::FileType::Mountable] {
        for validation_error in [
            None,
            Some(LocationValidationError::NotMounted(target.clone())),
            Some(LocationValidationError::Mountable(target.clone())),
        ] {
            for route in [
                "single-click",
                "activate",
                "in-place",
                "keyboard",
                "keyboard-in-place",
            ] {
                let browser = Browser::new(Rc::new(BrowseSource {
                    entry: entry_from_info(
                        Location::uri("smb://server/share-alias"),
                        browse_info(file_type, Some("smb://server/Documents"), LIST_ATTRIBUTES),
                    ),
                    validation_error: validation_error.clone(),
                }));
                let events = Rc::new(RefCell::new(Vec::new()));
                let observed = events.clone();
                browser.observe(move |event| observed.borrow_mut().push(event.clone()));
                browser.navigate(root.clone());
                events.borrow_mut().clear();

                match route {
                    "single-click" => browser.preview(0, 0),
                    "activate" => browser.activate(0, 0),
                    "in-place" => browser.activate_in_place(0, 0),
                    "keyboard" => {
                        browser.select(0, 0);
                        browser.activate_focused();
                    }
                    "keyboard-in-place" => {
                        browser.select(0, 0);
                        browser.activate_focused_in_place();
                    }
                    _ => unreachable!(),
                }

                let in_place = route.ends_with("in-place");
                if let Some(expected) = &validation_error {
                    assert_eq!(browser.active_location(), Some(root.clone()));
                    assert!(
                        events.borrow().iter().any(|event| match event {
                            BrowserEvent::NavigationRejected {
                                parent_depth: 0,
                                error,
                            } => {
                                !in_place && error == expected
                            }
                            BrowserEvent::LocationNavigationRejected { error } => {
                                in_place && error == expected
                            }
                            _ => false,
                        }),
                        "{file_type:?}, {route}, {expected:?}"
                    );
                } else {
                    assert_eq!(browser.active_location(), Some(target.clone()), "{route}");
                    assert_eq!(browser.active_depth(), Some(usize::from(!in_place)));
                    browser.back();
                    assert_eq!(browser.active_location(), Some(root.clone()));
                }
                assert!(!events.borrow().iter().any(|event| matches!(
                    event,
                    BrowserEvent::OpenRequested { .. } | BrowserEvent::PreviewRequested { .. }
                )));
            }
        }
    }
}
