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

fn recent_info(target: &str, recent_modified: i64) -> gio::FileInfo {
    let info = gio::FileInfo::new();
    info.set_name("recent-item");
    info.set_display_name("Recent item");
    info.set_file_type(gio::FileType::Regular);
    info.set_size(999);
    info.set_attribute_uint64(gio::FILE_ATTRIBUTE_TIME_MODIFIED, 999);
    info.set_attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI, target);
    info.set_attribute_int64(gio::FILE_ATTRIBUTE_RECENT_MODIFIED, recent_modified);
    info
}

fn target_info(name: &str, size: u64, modified: u64) -> gio::FileInfo {
    let info = gio::FileInfo::new();
    info.set_name(name);
    info.set_display_name(name);
    info.set_file_type(gio::FileType::Regular);
    info.set_size(size as i64);
    info.set_attribute_uint64(gio::FILE_ATTRIBUTE_TIME_MODIFIED, modified);
    info
}

#[test]
fn recent_entries_use_the_native_target_and_keep_recency_separate() {
    let recent = recent_info("file:///fixture/target.txt", 42);
    let target = recent_target_location(&recent).expect("the target URI should resolve");
    let entry = recent_entry_from_target(&recent, target, target_info("target.txt", 7, 123))
        .expect("the target should produce an operational entry");

    assert_eq!(entry.location, Location::local("/fixture/target.txt"));
    assert_eq!(entry.display_name, "target.txt");
    assert_eq!(entry.size, MetadataValue::Known(7));
    assert_eq!(entry.modified_unix_seconds, MetadataValue::Known(123));
    assert_eq!(entry.recent_unix_seconds, MetadataValue::Known(42));
}

#[test]
fn recent_entries_keep_non_native_target_uris_clean() {
    let recent = recent_info("smb://server/share/target.txt", 42);
    let target = recent_target_location(&recent).expect("the target URI should resolve");
    let entry = recent_entry_from_target(&recent, target, target_info("target.txt", 7, 123))
        .expect("the target should produce an operational entry");

    assert_eq!(
        entry.location,
        Location::uri("smb://server/share/target.txt")
    );
    assert!(entry.location.native_path().is_none());
}

#[test]
fn recent_virtual_targets_are_not_operational_entries() {
    let recent = recent_info("recent:///virtual-item", 42);

    assert_eq!(recent_target_location(&recent), None);
}

#[test]
fn recent_target_redirects_that_return_to_recent_are_not_operational_entries() {
    let recent = recent_info("file:///fixture/target-link", 42);
    let target_location = recent_target_location(&recent).expect("the target URI should resolve");
    let target_info = {
        let target = target_info("target-link", 7, 123);
        target.set_file_type(gio::FileType::Shortcut);
        target.set_attribute_string(
            gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI,
            "recent:///virtual-item",
        );
        target
    };

    assert!(recent_entry_from_target(&recent, target_location, target_info).is_none());
}

#[test]
fn unavailable_recent_targets_are_skipped() {
    let recent = recent_info("file:///fixture/missing-recent-target", 42);
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let resolution = glib::MainContext::default().block_on(resolve_recent_entry(
        recent,
        false,
        Instant::now() + Duration::from_secs(10),
    ));

    assert!(matches!(resolution, RecentEntryResolution::Stale));
}

struct BrowseSource {
    entry: FileEntry,
    validation_error: Option<LocationValidationError>,
}

impl FileSource for BrowseSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        assert_eq!(location, &self.entry.location);
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
    let target = location_for_file(&gio::File::for_uri("smb://server/Documents"))
        .expect("the browse target should be a valid location");
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
