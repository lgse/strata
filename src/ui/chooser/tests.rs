// SPDX-License-Identifier: MIT

mod acceptance;
mod context_menu;
mod filtered_preview;
mod keyboard;
mod layout;
mod selection;
mod sizing;

use std::{ffi::OsString, path::Path};

use ashpd::desktop::file_chooser::FileFilter;

use super::*;
use crate::model::{EntryKind, MetadataValue};

fn entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local(Path::new("/tmp").join(name)),
        native_name: OsString::from(name),
        thumbnail_path: None,
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
fn chooser_previews_the_same_supported_types_as_the_main_browser() {
    for name in [
        "photo.png",
        "clip.mp4",
        "song.ogg",
        "document.pdf",
        "notes.txt",
    ] {
        assert!(
            preview_target(Some(entry(name, EntryKind::File))).is_some(),
            "{name} should be previewable"
        );
    }
    assert!(preview_target(Some(entry("archive.zip", EntryKind::File))).is_none());
    assert!(
        preview_target(Some(entry("folder.mp4", EntryKind::Directory))).is_none(),
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

#[test]
fn chooser_dimensions_leave_margins_on_scaled_screens() {
    let (width, height) = chooser_default_dimensions_for_monitor(1152, 720);

    assert_eq!((width, height), (921, 561));
    assert!(1152 - width >= 120);
    assert!(720 - height >= 100);
}

#[test]
fn chooser_dimensions_have_maximums_on_large_screens() {
    assert_eq!(
        chooser_default_dimensions_for_monitor(1920, 1080),
        (MAX_CHOOSER_WIDTH, MAX_CHOOSER_HEIGHT)
    );
    assert_eq!(
        chooser_default_dimensions_for_monitor(2560, 1440),
        (MAX_CHOOSER_WIDTH, MAX_CHOOSER_HEIGHT)
    );
    assert_eq!(
        chooser_default_dimensions_for_monitor(i32::MAX, i32::MAX),
        (MAX_CHOOSER_WIDTH, MAX_CHOOSER_HEIGHT)
    );
}

#[test]
fn chooser_dimensions_adapt_to_compact_screens() {
    assert_eq!(
        chooser_default_dimensions_for_monitor(1024, 768),
        (819, 599)
    );
    assert_eq!(chooser_default_dimensions_for_monitor(800, 600), (640, 468));
    assert_eq!(chooser_default_dimensions_for_monitor(600, 400), (600, 400));
}

#[test]
fn chooser_dimensions_fall_back_for_invalid_geometry() {
    for geometry in [(0, 0), (-10, -20), (1920, 0), (0, 1080)] {
        assert_eq!(
            chooser_default_dimensions_for_monitor(geometry.0, geometry.1),
            (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT)
        );
    }
}

#[test]
fn chooser_dimensions_follow_split_application_geometry() {
    let monitor = Some((1920, 1080));
    assert_eq!(
        chooser_initial_dimensions(monitor, Some((960, 1080))),
        (768, 680)
    );
    assert_eq!(
        chooser_initial_dimensions(monitor, Some((960, 540))),
        (768, 460)
    );
    assert_eq!(
        chooser_initial_dimensions(monitor, Some((1920, 1080))),
        (1000, 680)
    );
    assert_eq!(
        chooser_initial_dimensions(monitor, Some((i32::MAX, i32::MAX))),
        (1000, 680)
    );
    assert_eq!(
        chooser_initial_dimensions(Some((800, 600)), Some((1920, 1080))),
        (640, 468)
    );
    assert_eq!(
        chooser_initial_dimensions(None, Some((960, 1080))),
        (768, 680)
    );
}

#[test]
fn missing_or_invalid_application_geometry_preserves_monitor_fallback() {
    for parent in [None, Some((0, 600)), Some((800, -1))] {
        assert_eq!(
            chooser_initial_dimensions(Some((1920, 1080)), parent),
            (1000, 680)
        );
        assert_eq!(chooser_initial_dimensions(None, parent), (920, 580));
        assert_eq!(
            chooser_initial_dimensions(Some((0, 1080)), parent),
            (920, 580)
        );
    }
}

#[test]
fn a_long_dropdown_opens_toward_the_roomier_side() {
    let (position, height) = dropdown_placement(680, 572, 602);
    assert_eq!(
        position,
        gtk::PositionType::Top,
        "a button near the bottom must open upward"
    );
    assert_eq!(height, 572 - DROPDOWN_EDGE_MARGIN);

    let (position, height) = dropdown_placement(680, 78, 108);
    assert_eq!(
        position,
        gtk::PositionType::Bottom,
        "a button near the top must open downward"
    );
    assert_eq!(height, 680 - 108 - DROPDOWN_EDGE_MARGIN);

    for (available, top, bottom) in [(0, 0, 0), (-10, -10, -5), (120, 60, 90)] {
        let (_, height) = dropdown_placement(available, top, bottom);
        assert_eq!(
            height, MIN_DROPDOWN_CONTENT_HEIGHT,
            "a cramped or invalid window still shows a usable list"
        );
    }
}

#[test]
fn a_long_dropdown_scrolls_instead_of_overflowing_the_window() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::a_long_dropdown_scrolls_instead_of_overflowing_the_window",
        || {
            let owned = (0..80)
                .map(|index| format!("Filter {index}"))
                .collect::<Vec<_>>();
            let labels = owned.iter().map(String::as_str).collect::<Vec<_>>();
            let dropdown = ChooserDropdown::new(&labels, 0);
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(561)
                .child(&dropdown.button)
                .build();
            window.present();
            dropdown.button.popup();

            let scroll = dropdown
                .popover
                .child()
                .and_downcast::<gtk::ScrolledWindow>()
                .expect("the dropdown list is scrollable");
            assert_eq!(scroll.vscrollbar_policy(), gtk::PolicyType::Automatic);
            assert!(scroll.propagates_natural_height());
            let bounded = scroll.max_content_height();
            assert!(
                bounded <= window.height().max(MIN_DROPDOWN_CONTENT_HEIGHT),
                "the list must stay within the window: {bounded}"
            );
            assert!(
                bounded >= MIN_DROPDOWN_CONTENT_HEIGHT,
                "the list must be bounded to a usable height, got {bounded}"
            );
            let natural = scroll
                .child()
                .expect("dropdown list")
                .preferred_size()
                .1
                .height();
            assert!(
                natural > bounded,
                "the fixture must exceed the bound so it actually scrolls: {natural} vs {bounded}"
            );
            window.destroy();
        },
    );
}
