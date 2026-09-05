// SPDX-License-Identifier: GPL-3.0-or-later

mod context_menu;
mod keyboard;
mod layout;
mod selection;

use std::{ffi::OsString, path::Path};

use ashpd::desktop::file_chooser::FileFilter;

use super::*;
use crate::model::{EntryKind, MetadataValue};

fn entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local(Path::new("/tmp").join(name)),
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
    }
}

#[test]
#[ignore = "requires a GTK display and exclusive main context"]
fn native_filters_match_globs_and_mime_types_without_hiding_directories() {
    gtk::init().expect("GTK display");
    let filter = gtk::FileFilter::new();
    filter.add_pattern("*.txt");
    filter.add_mime_type("image/jpeg");

    assert!(file_filter_matches(
        &filter,
        &entry("notes.txt", EntryKind::File)
    ));
    assert!(file_filter_matches(
        &filter,
        &entry("photo.jpg", EntryKind::File)
    ));
    assert!(!file_filter_matches(
        &filter,
        &entry("archive.zip", EntryKind::File)
    ));
    assert!(file_filter_matches(
        &filter,
        &entry("folder.zip", EntryKind::Directory)
    ));
    let hidden = entry("archive.zip", EntryKind::File);
    assert!(
        matches!(filter_directory_change(Some(&filter), DirectoryChange::Upsert(hidden.clone())), DirectoryChange::Remove(location) if location == hidden.location)
    );
    let previous = Location::local("/tmp/previous.txt");
    assert!(
        matches!(filter_directory_change(Some(&filter), DirectoryChange::Move { from: previous.clone(), entry: hidden }), DirectoryChange::Remove(location) if location == previous)
    );
}

#[test]
fn portal_filters_select_the_requested_filter_or_the_first_one() {
    let images = FileFilter::new("Images").glob("*.png");
    let text = FileFilter::new("Text").glob("*.txt");

    let (filters, selected) = normalize_portal_filters(std::slice::from_ref(&images), None);
    assert_eq!(selected, Some(0));
    assert_eq!(filters[0], images);

    let (filters, selected) = normalize_portal_filters(std::slice::from_ref(&images), Some(&text));
    assert_eq!(selected, Some(1));
    assert_eq!(filters[1], text);
}

#[test]
fn chooser_locations_reject_remote_uris_before_io() {
    let source = ChooserFileSource::new();
    let error = source
        .validate_location(&Location::uri("smb://server/share"))
        .expect_err("remote locations are unavailable");
    assert!(matches!(
        error,
        LocationValidationError::UnsupportedScheme(_)
    ));
    assert!(error.to_string().contains("local files and folders only"));
}

#[test]
fn chooser_watches_local_directories() {
    let root = tempfile::tempdir().expect("temporary directory");
    let source = ChooserFileSource::new();

    let watch = source.watch(Location::local(root.path()), true, Rc::new(|_| {}));

    assert!(watch.is_some());
}

#[test]
fn chooser_fills_metadata_for_the_current_browser_views() {
    use crate::services::{MetadataOutcome, RequestId};
    use std::time::Duration;

    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context test lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("exclusive metadata main context");
    let root = tempfile::tempdir().expect("fixture directory");
    let path = root.path().join("notes.txt");
    std::fs::write(&path, b"notes").expect("metadata fixture");
    let source = ChooserFileSource::new();
    assert!(source.supports_metadata_fill(&Location::local(root.path())));
    assert!(!source.supports_metadata_fill(&Location::uri("smb://server/share")));
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    let _load = source.fill_metadata(
        MetadataRequest {
            id: RequestId(1),
            entries: vec![Location::local(&path)],
            full: false,
            time_budget: Duration::from_secs(2),
        },
        Rc::new(move |event| received.borrow_mut().push(event)),
    );
    context.block_on(async {
        glib::future_with_timeout(Duration::from_secs(5), async {
            while !events
                .borrow()
                .iter()
                .any(|event| matches!(event, DirectoryEvent::MetadataFinished { .. }))
            {
                glib::timeout_future(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("metadata fill completes within the deadline");
    });
    let events = events.borrow();
    assert!(events.iter().any(|event| matches!(event, DirectoryEvent::MetadataFilled { updates, .. } if updates.iter().any(|update| update.size == MetadataValue::Known(5)))));
    assert!(matches!(
        events.last(),
        Some(DirectoryEvent::MetadataFinished {
            outcome: MetadataOutcome::Complete,
            ..
        })
    ));
}

#[test]
fn chooser_async_validation_rejects_remote_locations() {
    let source = ChooserFileSource::new();
    let result = Rc::new(RefCell::new(None));
    let received = result.clone();
    let _load = source.validate_location_async(
        Location::uri("smb://server/share"),
        Rc::new(move |value| {
            received.replace(Some(value));
        }),
    );
    assert!(matches!(
        result.take(),
        Some(Err(LocationValidationError::UnsupportedScheme(_)))
    ));
}

#[test]
fn chooser_previews_supported_files_but_not_directories() {
    assert!(
        chooser_preview_target(Some(entry("notes.txt", EntryKind::File))).is_some(),
        "supported files should be previewable"
    );
    assert!(
        chooser_preview_target(Some(entry("folder", EntryKind::Directory))).is_none(),
        "folders should remain navigation targets"
    );
}

#[test]
fn chooser_selection_excludes_navigation_items() {
    let file = entry("notes.txt", EntryKind::File);
    let folder = entry("folder", EntryKind::Directory);

    assert_eq!(
        eligible_open_entries(vec![folder.clone(), file.clone()], false),
        [file]
    );
    assert_eq!(
        eligible_open_entries(vec![folder.clone(), folder.clone()], true),
        [folder.clone(), folder]
    );
}

#[test]
fn folder_accept_shortcut_requires_control_and_enter() {
    let control = gtk::gdk::ModifierType::CONTROL_MASK;
    let shift = gtk::gdk::ModifierType::SHIFT_MASK;
    let alt = gtk::gdk::ModifierType::ALT_MASK;

    assert!(is_folder_accept_shortcut(gtk::gdk::Key::Return, control));
    assert!(is_folder_accept_shortcut(gtk::gdk::Key::KP_Enter, control));
    assert!(!is_folder_accept_shortcut(
        gtk::gdk::Key::Return,
        gtk::gdk::ModifierType::empty()
    ));
    assert!(!is_folder_accept_shortcut(
        gtk::gdk::Key::Return,
        control | shift
    ));
    assert!(!is_folder_accept_shortcut(
        gtk::gdk::Key::Return,
        control | alt
    ));
}
