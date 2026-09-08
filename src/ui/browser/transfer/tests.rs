// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{FileEntry, Location};
use std::path::Path;

#[test]
fn duplicate_transfer_uses_the_selected_entries_parent() {
    let entry = |path: &str| FileEntry {
        location: Location::local(path),
        native_name: Path::new(path).file_name().unwrap_or_default().to_owned(),
        thumbnail_path: None,
        display_name: path.to_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        mode: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let first = entry("/fixture/selected/first.txt");
    let second = entry("/fixture/selected/second.txt");

    assert_eq!(
        duplicate_transfer(&[first.clone(), second.clone()]),
        Some((
            Location::local("/fixture/selected"),
            vec![first.location, second.location]
        ))
    );
    assert_eq!(
        duplicate_transfer(&[entry("/fixture/one.txt"), entry("/other/two.txt")]),
        None
    );
    assert_eq!(duplicate_transfer(&[]), None);
    for uri in ["trash:///file.txt", "trash:///folder/file.txt"] {
        let trashed = FileEntry {
            location: Location::uri(uri),
            ..entry("file.txt")
        };
        assert_eq!(duplicate_transfer(&[trashed]), None);
    }
}

#[test]
fn transfer_collisions_detect_existing_destination_items() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!("strata-collision-test-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let destination = root.join("destination");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::create_dir_all(&destination)?;
    let source = source_dir.join("photo.jpg");
    std::fs::write(&source, b"new")?;

    assert!(!transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));
    assert!(!transfer_has_collision(
        &Location::local(&source),
        &Location::local(&source_dir)
    ));
    std::fs::write(destination.join("photo.jpg"), b"old")?;
    assert!(transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn start_transfer_skips_noops_before_emitting_progress() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::start_transfer_skips_noops_before_emitting_progress",
        || {
            for moving in [false, true] {
                let fixture = tempfile::tempdir().expect("transfer fixture");
                let source_path = fixture.path().join("source");
                let nested_path = source_path.join("nested");
                let other_path = fixture.path().join("notes.txt");
                std::fs::create_dir_all(&nested_path).expect("nested source folder");
                std::fs::write(&other_path, "notes").expect("source file");
                let source = Location::local(&source_path);
                let view = crate::ui::browser::BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                let browser = view.browser();
                let started = Rc::new(RefCell::new(Vec::new()));
                let finished = Rc::new(Cell::new(false));
                let observed_started = started.clone();
                let observed_finished = finished.clone();
                browser.observe(move |event| match event {
                    crate::app::BrowserEvent::TransferStarted { total, moving } => {
                        observed_started.borrow_mut().push((*total, *moving));
                    }
                    crate::app::BrowserEvent::TransferFinished { .. } => {
                        observed_finished.set(true);
                    }
                    crate::app::BrowserEvent::OperationFailed { message } => {
                        panic!("transfer failed: {message}");
                    }
                    _ => {}
                });

                for destination in [source.clone(), Location::local(&nested_path)] {
                    view.start_transfer(destination, vec![source.clone()], moving);
                }
                view.start_transfer(
                    Location::local(fixture.path()),
                    vec![source.clone(), Location::local(&other_path)],
                    true,
                );
                assert!(started.borrow().is_empty());
                assert!(!finished.get());

                view.start_transfer(
                    source.clone(),
                    vec![source, Location::local(&other_path)],
                    moving,
                );
                assert_eq!(*started.borrow(), vec![(1, moving)]);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while !finished.get() {
                    assert!(std::time::Instant::now() < deadline, "transfer timed out");
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                assert_eq!(
                    std::fs::read_to_string(source_path.join("notes.txt")).expect("copied notes"),
                    "notes"
                );
                assert_eq!(other_path.exists(), !moving);
                assert!(nested_path.is_dir());
                assert!(!source_path.join("source").exists());
                browser.clear_observer();
            }
        },
    );
}

#[test]
fn transfer_noops_preserve_same_folder_copies() {
    for root in [
        Location::local("/fixture"),
        Location::uri("file:///fixture"),
        Location::uri("sftp://example.test/fixture"),
    ] {
        let root_file = gio_file_for_location(&root);
        let source = Location::uri(root_file.child("source").uri());
        let nested = Location::uri(root_file.child("source/nested").uri());
        let sibling = Location::uri(root_file.child("source-other").uri());
        let file = Location::uri(root_file.child("photo.jpg").uri());
        for moving in [false, true] {
            assert!(transfer_is_noop(&source, &source, moving));
            assert!(transfer_is_noop(&source, &nested, moving));
            assert!(!transfer_is_noop(&source, &sibling, moving));
            assert_eq!(transfer_is_noop(&source, &root, moving), moving);
            assert_eq!(transfer_is_noop(&file, &root, moving), moving);
        }
    }
}
