// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{FileEntry, Location};
use crate::services::{DropCommit, TransferKind};
use std::path::Path;

fn visible_texts(overlay: &gtk::Overlay) -> Vec<String> {
    let mut texts = Vec::new();
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if !widget.is_visible() {
            continue;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            texts.push(label.label().to_string());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && let Some(label) = button.label()
        {
            texts.push(label.to_string());
        }
        let mut descendant = widget.first_child();
        while let Some(child) = descendant {
            stack.push(child.clone());
            descendant = child.next_sibling();
        }
    }
    texts
}

fn has_visible_button(overlay: &gtk::Overlay, label: &str) -> bool {
    visible_texts(overlay).iter().any(|text| text == label)
}

fn button_with_label(overlay: &gtk::Overlay, label: &str) -> Option<gtk::Button> {
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(label)
            && button.is_visible()
        {
            return Some(button.clone());
        }
        let mut descendant = widget.first_child();
        while let Some(next) = descendant {
            stack.push(next.clone());
            descendant = next.next_sibling();
        }
    }
    None
}

fn click_button(overlay: &gtk::Overlay, label: &str) {
    button_with_label(overlay, label)
        .unwrap_or_else(|| panic!("visible {label:?} button not found"))
        .emit_clicked();
}

fn wait_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn wait_for_modal_layer(overlay: &gtk::Overlay) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut child = overlay.first_child();
        while let Some(widget) = child {
            if widget.has_css_class("app-modal-layer") {
                return true;
            }
            child = widget.next_sibling();
        }
    }
    false
}

fn transfer_entry(path: &Path) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: path.file_name().unwrap_or_default().to_owned(),
        thumbnail_path: None,
        display_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn destination_field(overlay: &gtk::Overlay) -> gtk::Entry {
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if let Some(field) = widget.downcast_ref::<gtk::Entry>()
            && field.placeholder_text().as_deref() == Some("Search for a folder…")
        {
            return field.clone();
        }
        let mut descendant = widget.first_child();
        while let Some(child) = descendant {
            stack.push(child.clone());
            descendant = child.next_sibling();
        }
    }
    panic!("destination field was not found");
}

fn visible_error_message(overlay: &gtk::Overlay) -> Option<String> {
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.is_visible()
            && label.has_css_class("error")
        {
            return Some(label.text().to_string());
        }
        let mut descendant = widget.first_child();
        while let Some(child) = descendant {
            stack.push(child.clone());
            descendant = child.next_sibling();
        }
    }
    None
}

fn find_widget_with_class(overlay: &gtk::Overlay, class: &str) -> Option<gtk::Widget> {
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) {
            return Some(widget);
        }
        let mut descendant = widget.first_child();
        while let Some(next) = descendant {
            stack.push(next.clone());
            descendant = next.next_sibling();
        }
    }
    None
}

#[expect(
    deprecated,
    reason = "GTK 4.10 deprecated style_context lookup without a public replacement for reading a widget's resolved theme color"
)]
fn resolved_dialog_surface(overlay: &gtk::Overlay) -> String {
    let dialog = find_widget_with_class(overlay, "action-dialog").expect("conflict dialog");
    let color = dialog
        .style_context()
        .lookup_color("theme_surface")
        .unwrap_or_else(|| {
            panic!("the dialog style context must resolve the active theme surface")
        });
    color.to_string()
}

#[test]
fn drop_commit_kind_describes_the_pending_cursor_action() {
    assert_eq!(DropCommit::Copy.transfer_kind(), TransferKind::Copy);
    assert_eq!(DropCommit::Move.transfer_kind(), TransferKind::Move);
    assert_eq!(
        DropCommit::Ask {
            default: TransferKind::Copy,
            volume: VolumeRelation::Different,
        }
        .transfer_kind(),
        TransferKind::Copy
    );
}

#[test]
fn cross_volume_prompt_only_claims_another_device_when_the_lookup_resolved() {
    assert_eq!(
        cross_volume_drop_description(VolumeRelation::Different),
        "The destination is on a different device."
    );
    assert_eq!(
        cross_volume_drop_description(VolumeRelation::Unknown),
        "Strata could not determine whether the destination is on the same device."
    );
}

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
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
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
            image_dimensions: crate::model::MetadataValue::Unknown,
            child_count: crate::model::MetadataValue::Unknown,
            duration_seconds: crate::model::MetadataValue::Unknown,
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

    assert!(
        transfer_collision(&Location::local(&source), &Location::local(&destination)).is_none()
    );
    assert!(transfer_collision(&Location::local(&source), &Location::local(&source_dir)).is_none());
    std::fs::write(destination.join("photo.jpg"), b"old")?;
    let collision = transfer_collision(&Location::local(&source), &Location::local(&destination))
        .expect("a file collision");
    assert!(!collision.mergeable, "a file collision cannot merge");

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn transfer_collisions_mark_folder_pairs_as_mergeable() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("strata-mergeable-test-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let destination = root.join("destination");
    std::fs::create_dir_all(source_dir.join("folder"))?;
    std::fs::create_dir_all(&destination)?;
    std::fs::write(source_dir.join("file.txt"), b"new")?;

    let folder = transfer_collision(
        &Location::local(source_dir.join("folder")),
        &Location::local(&destination),
    );
    assert!(
        folder.is_none(),
        "no collision until the destination exists"
    );

    std::fs::create_dir_all(destination.join("folder"))?;
    std::fs::write(destination.join("file.txt"), b"old")?;
    let folder = transfer_collision(
        &Location::local(source_dir.join("folder")),
        &Location::local(&destination),
    )
    .expect("a folder collision");
    assert!(folder.mergeable, "two folders can merge");
    let file = transfer_collision(
        &Location::local(source_dir.join("file.txt")),
        &Location::local(&destination),
    )
    .expect("a file collision");
    assert!(!file.mergeable);

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
                    view.state
                        .start_transfer(destination, vec![source.clone()], moving);
                }
                view.state.start_transfer(
                    Location::local(fixture.path()),
                    vec![source.clone(), Location::local(&other_path)],
                    true,
                );
                assert!(started.borrow().is_empty());
                assert!(!finished.get());

                view.state.start_transfer(
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
fn send_to_copies_every_source_without_reveal_or_navigation() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_copies_every_source_without_reveal_or_navigation",
        || {
            let fixture = tempfile::tempdir().expect("send-to fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("device-root");
            let previous_mount_root = fixture.path().join("previous-mount-root");
            std::fs::create_dir(&source_dir).expect("source directory");
            std::fs::create_dir(&destination).expect("device root");
            std::fs::create_dir(&previous_mount_root).expect("previous mount root");
            let single = source_dir.join("single.txt");
            let first = source_dir.join("first.txt");
            let second = source_dir.join("second.txt");
            for (path, contents) in [(&single, "single"), (&first, "first"), (&second, "second")] {
                std::fs::write(path, contents).expect("source file");
            }

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.navigate_location(Location::local(&source_dir));
            let browser = view.browser();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            browser.observe(move |event| observed.borrow_mut().push(event.clone()));

            view.state.send_to_removable_device_with_resolver(
                "volume:current-device",
                vec![Location::local(&single)],
                |id| {
                    assert_eq!(id, "volume:current-device");
                    Some(destination.clone())
                },
            );
            wait_until(
                || {
                    events.borrow().iter().any(|event| {
                        matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                    })
                },
                "single-item send-to completion",
            );
            view.state.send_to(
                Location::local(&destination),
                vec![Location::local(&first), Location::local(&second)],
                SendToTransferContext {
                    device_name: "test-device".to_owned(),
                },
            );
            wait_until(
                || {
                    events
                        .borrow()
                        .iter()
                        .filter(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                        .count()
                        == 2
                },
                "multi-selection send-to completion",
            );

            for (path, contents) in [
                (destination.join("single.txt"), "single"),
                (destination.join("first.txt"), "first"),
                (destination.join("second.txt"), "second"),
            ] {
                assert_eq!(
                    std::fs::read_to_string(path).expect("copied item"),
                    contents
                );
            }
            assert!(single.exists() && first.exists() && second.exists());
            assert!(
                std::fs::read_dir(&previous_mount_root)
                    .expect("previous mount root")
                    .next()
                    .is_none(),
                "the menu-time mount root is not used"
            );
            assert_eq!(
                browser.active_location(),
                Some(Location::local(&source_dir))
            );
            let events = events.borrow();
            assert_eq!(
                events
                    .iter()
                    .filter_map(|event| match event {
                        crate::app::BrowserEvent::TransferStarted { total, moving } => {
                            Some((*total, *moving))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                [(1, false), (2, false)]
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, crate::app::BrowserEvent::TransferReveal { .. }))
            );
            browser.clear_observer();
        },
    );
}

#[test]
fn send_to_drive_root_uses_current_root_after_same_id_remount() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_drive_root_uses_current_root_after_same_id_remount",
        || {
            let fixture = tempfile::tempdir().expect("remount fixture");
            let source_dir = fixture.path().join("source");
            let stale_root = fixture.path().join("mount-a");
            let current_root = fixture.path().join("mount-b");
            std::fs::create_dir(&source_dir).expect("source directory");
            std::fs::create_dir(&stale_root).expect("stale mount root");
            std::fs::create_dir(&current_root).expect("current mount root");
            let source = source_dir.join("selected.txt");
            std::fs::write(&source, "selected contents").expect("source file");

            let device_id = "volume:remounted-device";
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.navigate_location(Location::local(&source_dir));
            let browser = view.browser();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            browser.observe(move |event| observed.borrow_mut().push(event.clone()));

            // The same stable ID now resolves to the remounted root. Activation
            // must use the current root; the stale menu-time root is not
            // authoritative.
            let current = current_root.clone();
            view.state.send_to_removable_device_with_resolver(
                device_id,
                vec![Location::local(&source)],
                |resolved_id| {
                    assert_eq!(resolved_id, device_id);
                    Some(current.clone())
                },
            );
            wait_until(
                || {
                    current_root.join("selected.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "drive-root copy to the current mount root",
            );

            assert_eq!(
                std::fs::read_to_string(current_root.join("selected.txt"))
                    .expect("copy on remounted device"),
                "selected contents"
            );
            assert!(source.exists(), "Send to preserves the selected source");
            assert!(
                std::fs::read_dir(&stale_root)
                    .expect("stale mount root")
                    .next()
                    .is_none(),
                "the stale root is not trusted after remount"
            );
            assert_eq!(
                browser.active_location(),
                Some(Location::local(&source_dir)),
                "Send to leaves the browser location unchanged"
            );
            let events = events.borrow();
            assert_eq!(
                events
                    .iter()
                    .filter_map(|event| match event {
                        crate::app::BrowserEvent::TransferStarted { total, moving } => {
                            Some((*total, *moving))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                [(1, false)]
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, crate::app::BrowserEvent::TransferReveal { .. })),
                "Send to never reveals its destination"
            );
            browser.clear_observer();
        },
    );
}

#[test]
fn choose_folder_rejects_invalid_destinations_and_copies_into_a_confined_directory() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_rejects_invalid_destinations_and_copies_into_a_confined_directory",
        || {
            let fixture = tempfile::tempdir().expect("choose-folder fixture");
            let source_dir = fixture.path().join("source");
            let device = fixture.path().join("device");
            let outside = fixture.path().join("outside");
            std::fs::create_dir_all(&source_dir).expect("source directory");
            std::fs::create_dir_all(device.join("subdir")).expect("device subdirectory");
            std::fs::create_dir_all(&outside).expect("outside directory");
            let first = source_dir.join("first.txt");
            let second = source_dir.join("second.txt");
            for path in [&first, &second] {
                let name = path
                    .file_name()
                    .expect("source file name")
                    .to_string_lossy();
                std::fs::write(path, name.as_bytes()).expect("source file");
            }
            std::fs::write(device.join("regular-file.txt"), b"file").expect("device file");
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&outside, device.join("external-link"))
                    .expect("external symlink");
                std::os::unix::fs::symlink(device.join("subdir"), device.join("internal-link"))
                    .expect("internal symlink");
            }

            let device_id = "volume:current-device";
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.remember_send_to_destination(device_id, Path::new("Previously used"), None);
            let original_recents = preferences.send_to_recent_destinations(device_id);
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.navigate_location(Location::local(&source_dir));
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            view.browser()
                .observe(move |event| observed.borrow_mut().push(event.clone()));

            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&first), Location::local(&second)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            assert!(wait_for_modal_layer(&overlay), "Choose folder dialog opens");
            let field = destination_field(&overlay);
            assert_eq!(
                field.text(),
                folder_input_path(&device),
                "the field starts at the current device root"
            );
            let invalid = [
                device.join("missing"),
                device.join("regular-file.txt"),
                outside.clone(),
                device.join("../outside"),
            ];
            for path in invalid {
                field.set_text(&path.to_string_lossy());
                click_button(&overlay, "Copy here");
                wait_until(
                    || visible_error_message(&overlay).is_some(),
                    "the invalid destination error",
                );
                assert!(wait_for_modal_layer(&overlay), "the dialog stays open");
                assert!(button_with_label(&overlay, "Create and copy").is_none());
                assert!(
                    events.borrow().iter().all(|event| !matches!(
                        event,
                        crate::app::BrowserEvent::TransferStarted { .. }
                    )),
                    "an invalid destination is rejected before dispatch"
                );
                assert_eq!(
                    preferences.send_to_recent_destinations(device_id),
                    original_recents,
                    "failed path validation does not change recent destinations"
                );
            }
            assert!(!device.join("missing").exists());
            #[cfg(unix)]
            {
                field.set_text(&device.join("external-link").to_string_lossy());
                click_button(&overlay, "Copy here");
                wait_until(
                    || visible_error_message(&overlay).is_some(),
                    "the symlink escape error",
                );
                assert!(wait_for_modal_layer(&overlay), "the dialog stays open");
                assert!(
                    events.borrow().iter().all(|event| !matches!(
                        event,
                        crate::app::BrowserEvent::TransferStarted { .. }
                    )),
                    "a symlink escape is rejected before dispatch"
                );
                assert_eq!(
                    preferences.send_to_recent_destinations(device_id),
                    original_recents,
                    "a rejected symlink escape does not change recent destinations"
                );
            }

            #[cfg(unix)]
            let selected = device.join("internal-link");
            #[cfg(not(unix))]
            let selected = device.join("subdir");
            field.set_text(&selected.to_string_lossy());
            click_button(&overlay, "Copy here");
            wait_until(
                || {
                    ["first.txt", "second.txt"]
                        .iter()
                        .all(|name| device.join("subdir").join(name).exists())
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the confined multi-source copy",
            );
            assert!(first.exists() && second.exists(), "sources remain in place");
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(&source_dir)),
                "Send to does not navigate to its destination"
            );
            assert!(
                events
                    .borrow()
                    .iter()
                    .all(|event| !matches!(event, crate::app::BrowserEvent::TransferReveal { .. })),
                "Send to never reveals its destination"
            );
            assert!(view.state.pending_navigate.borrow().is_none());
            assert!(view.state.pending_select.borrow().is_empty());
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn choose_folder_revalidates_device_while_the_dialog_is_open() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_revalidates_device_while_the_dialog_is_open",
        || {
            let fixture = tempfile::tempdir().expect("removed-device fixture");
            let source = fixture.path().join("source.txt");
            let device = fixture.path().join("device");
            std::fs::write(&source, b"source").expect("source file");
            std::fs::create_dir_all(device.join("folder")).expect("device folder");
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.remember_send_to_destination(
                "volume:removed-device",
                Path::new("Previously used"),
                None,
            );
            let original_recents = preferences.send_to_recent_destinations("volume:removed-device");
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            view.browser()
                .observe(move |event| observed.borrow_mut().push(event.clone()));
            let current = Rc::new(RefCell::new(Some(device.clone())));
            let resolved = current.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:removed-device".to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |id| {
                    (id == "volume:removed-device")
                        .then(|| resolved.borrow().clone())
                        .flatten()
                }),
            );
            assert!(wait_for_modal_layer(&overlay), "Choose folder dialog opens");
            let field = destination_field(&overlay);
            field.set_text(&device.join("folder").to_string_lossy());
            current.replace(None);
            click_button(&overlay, "Copy here");
            wait_until(
                || {
                    visible_error_message(&overlay).as_deref()
                        == Some("The removable device is no longer available.")
                },
                "the unavailable device error",
            );
            assert!(wait_for_modal_layer(&overlay), "the dialog stays open");
            assert!(
                events.borrow().iter().all(|event| !matches!(
                    event,
                    crate::app::BrowserEvent::TransferStarted { .. }
                )),
                "a removed device cannot dispatch a transfer"
            );
            assert!(!device.join("folder/source.txt").exists());
            assert_eq!(
                preferences.send_to_recent_destinations("volume:removed-device"),
                original_recents,
                "a device removed while the chooser is open does not change its recent destinations"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn choose_folder_uses_the_current_root_after_a_remount() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_uses_the_current_root_after_a_remount",
        || {
            let fixture = tempfile::tempdir().expect("remount fixture");
            let source = fixture.path().join("source.txt");
            let opened_root = fixture.path().join("mount-a");
            let current_root = fixture.path().join("mount-b");
            std::fs::write(&source, b"source").expect("source file");
            std::fs::create_dir_all(opened_root.join("nested")).expect("folder on initial root");
            std::fs::create_dir_all(&current_root).expect("current root");
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            view.browser()
                .observe(move |event| observed.borrow_mut().push(event.clone()));
            let current = Rc::new(RefCell::new(Some(opened_root.clone())));
            let resolved = current.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:remounted-device".to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |_| resolved.borrow().clone()),
            );
            assert!(wait_for_modal_layer(&overlay), "Choose folder dialog opens");
            let field = destination_field(&overlay);
            field.set_text(&opened_root.join("nested").to_string_lossy());
            current.replace(Some(current_root.clone()));
            std::fs::remove_dir_all(&opened_root).expect("old mount removed");
            click_button(&overlay, "Copy here");
            wait_until(
                || visible_error_message(&overlay).is_some(),
                "the missing corresponding folder error",
            );
            assert!(!current_root.join("nested").exists());
            assert!(
                events.borrow().iter().all(|event| !matches!(
                    event,
                    crate::app::BrowserEvent::TransferStarted { .. }
                )),
                "a missing relative folder on the new mount is not created"
            );

            std::fs::create_dir(current_root.join("nested"))
                .expect("corresponding folder on current mount");
            click_button(&overlay, "Copy here");
            wait_until(
                || {
                    current_root.join("nested/source.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the transfer to the current mount root",
            );
            assert!(!opened_root.exists());
            assert!(source.exists());
            assert!(
                events
                    .borrow()
                    .iter()
                    .all(|event| !matches!(event, crate::app::BrowserEvent::TransferReveal { .. })),
                "the remounted destination is not revealed"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn choose_folder_does_not_open_when_device_resolution_fails() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_does_not_open_when_device_resolution_fails",
        || {
            let source = Location::local("/tmp/source.txt");
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:missing-device".to_owned(),
                vec![source],
                Rc::new(|_| None),
            );
            wait_until(
                || has_visible_button(&overlay, "Close"),
                "the unavailable destination dialog",
            );
            assert!(has_visible_button(&overlay, "Destination unavailable"));
            assert!(button_with_label(&overlay, "Copy here").is_none());
            click_button(&overlay, "Close");
            window.destroy();
        },
    );
}

#[test]
fn choose_folder_breadcrumbs_navigate_to_ancestor() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_breadcrumbs_navigate_to_ancestor",
        || {
            let fixture = tempfile::tempdir().expect("breadcrumb fixture");
            let source_dir = fixture.path().join("source");
            let device = fixture.path().join("device");
            std::fs::create_dir_all(&source_dir).expect("source directory");
            std::fs::create_dir_all(device.join("Teaching")).expect("device subdirectory");
            let source = source_dir.join("source.txt");
            std::fs::write(&source, b"source").expect("source file");

            let device_id = "volume:breadcrumb-device";
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.navigate_location(Location::local(&source_dir));
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            view.browser()
                .observe(move |event| observed.borrow_mut().push(event.clone()));

            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            assert!(wait_for_modal_layer(&overlay), "Choose folder dialog opens");
            let field = destination_field(&overlay);
            assert_eq!(
                field.text(),
                folder_input_path(&device),
                "the field starts at the current device root"
            );
            // Descend with the existing suggestion row, then return with the
            // ancestor breadcrumb instead of editing the path.
            wait_until(
                || find_widget_with_class(&overlay, "transfer-suggestion").is_some(),
                "the device-root suggestions",
            );
            find_widget_with_class(&overlay, "transfer-suggestion")
                .expect("device-root suggestion")
                .downcast::<gtk::Button>()
                .expect("suggestion row")
                .emit_clicked();
            let teaching = device.join("Teaching");
            wait_until(
                || field.text() == folder_input_path(&teaching),
                "the suggestion fills the entry with the child folder",
            );
            let crumb = button_with_label(&overlay, "device").expect("device-root breadcrumb");
            crumb.emit_clicked();
            wait_until(
                || field.text() == folder_input_path(&device),
                "the ancestor breadcrumb returns the entry to the device root",
            );
            click_button(&overlay, "Copy here");
            wait_until(
                || {
                    device.join("source.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the copy into the breadcrumb-selected destination",
            );
            assert!(source.exists(), "sources remain in place");
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(&source_dir)),
                "Send to does not navigate to its destination"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn choose_folder_location_bar_switches_presentations() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_location_bar_switches_presentations",
        || {
            let fixture = tempfile::tempdir().expect("location bar fixture");
            let source_dir = fixture.path().join("source");
            let device = fixture.path().join("device");
            std::fs::create_dir_all(&source_dir).expect("source directory");
            std::fs::create_dir_all(device.join("Teaching")).expect("device subdirectory");
            let source = source_dir.join("source.txt");
            std::fs::write(&source, b"source").expect("source file");

            let device_id = "volume:location-bar-device";
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.navigate_location(Location::local(&source_dir));
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            view.browser()
                .observe(move |event| observed.borrow_mut().push(event.clone()));

            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            assert!(wait_for_modal_layer(&overlay), "Choose folder dialog opens");
            let field = destination_field(&overlay);
            let stack = find_widget_with_class(&overlay, "destination-location-stack")
                .expect("location stack")
                .downcast::<gtk::Stack>()
                .expect("stack widget");
            let visible_child = || {
                stack
                    .visible_child_name()
                    .as_deref()
                    .unwrap_or_default()
                    .to_owned()
            };
            assert_eq!(visible_child(), "browse");
            // Entering edit through the current crumb preserves the path.
            click_button(&overlay, "device");
            assert_eq!(visible_child(), "edit");
            assert_eq!(
                field.text(),
                folder_input_path(&device),
                "entering edit preserves the path"
            );
            // Activating a suggestion returns to browse with the child path.
            wait_until(
                || find_widget_with_class(&overlay, "transfer-suggestion").is_some(),
                "the device-root suggestions",
            );
            find_widget_with_class(&overlay, "transfer-suggestion")
                .expect("device-root suggestion")
                .downcast::<gtk::Button>()
                .expect("suggestion row")
                .emit_clicked();
            let teaching = device.join("Teaching");
            wait_until(
                || field.text() == folder_input_path(&teaching),
                "the suggestion fills the entry with the child folder",
            );
            assert_eq!(visible_child(), "browse");
            // Ancestor navigation still works from browse mode.
            click_button(&overlay, "device");
            wait_until(
                || field.text() == folder_input_path(&device),
                "the ancestor breadcrumb returns the entry to the device root",
            );
            // Enter still confirms through the stacked entry.
            field.emit_by_name::<()>("activate", &[]);
            wait_until(
                || {
                    device.join("source.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "Enter confirms the breadcrumb-selected destination",
            );
            assert!(source.exists(), "sources remain in place");
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

fn focused_enter_destination_fixture(
    name: &str,
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let fixture = tempfile::tempdir().expect(name);
    let source_dir = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    std::fs::create_dir_all(&source_dir).expect("source directory");
    std::fs::create_dir_all(&destination).expect("destination directory");
    std::fs::write(source_dir.join("photo.txt"), b"photo").expect("source file");
    (fixture, source_dir, destination)
}

fn destination_entry_owns_focus(field: &gtk::Entry, window: &gtk::Window) -> bool {
    // GtkEntry delegates keyboard focus to its internal GtkText, so the
    // entry itself never reports focused; match toplevel focus instead.
    gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| focus.is_ancestor(field))
}

fn open_transfer_browser(
    source_dir: &std::path::Path,
) -> (
    crate::ui::browser::BrowserView,
    gtk::Overlay,
    gtk::Window,
    Rc<RefCell<Vec<crate::app::BrowserEvent>>>,
) {
    let view = crate::ui::browser::BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.navigate_location(Location::local(source_dir));
    let overlay = view.overlay();
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    view.browser()
        .observe(move |event| observed.borrow_mut().push(event.clone()));
    (view, overlay, window, events)
}

#[test]
fn copy_to_focused_enter_confirms_destination() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::copy_to_focused_enter_confirms_destination",
        || {
            let (_fixture, source_dir, destination) =
                focused_enter_destination_fixture("copy focused-enter fixture");
            let source = source_dir.join("photo.txt");
            let (view, overlay, window, events) = open_transfer_browser(&source_dir);
            view.state
                .show_transfer_dialog(vec![transfer_entry(&source)], false);
            assert!(wait_for_modal_layer(&overlay), "Copy dialog opens");
            let field = destination_field(&overlay);
            click_button(&overlay, "source");
            wait_until(
                || destination_entry_owns_focus(&field, &window),
                "the entry owns focus on the Enter path",
            );
            field.set_text(&destination.to_string_lossy());
            field.emit_by_name::<()>("activate", &[]);
            wait_until(
                || {
                    destination.join("photo.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "focused Enter copies to the destination",
            );
            assert!(source.exists(), "Copy leaves its source");
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(&destination)),
                "Copy navigates to its destination"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn move_to_focused_enter_confirms_destination() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::move_to_focused_enter_confirms_destination",
        || {
            let (_fixture, source_dir, destination) =
                focused_enter_destination_fixture("move focused-enter fixture");
            let source = source_dir.join("photo.txt");
            let (view, overlay, window, events) = open_transfer_browser(&source_dir);
            view.state
                .show_transfer_dialog(vec![transfer_entry(&source)], true);
            assert!(wait_for_modal_layer(&overlay), "Move dialog opens");
            let field = destination_field(&overlay);
            click_button(&overlay, "source");
            wait_until(
                || destination_entry_owns_focus(&field, &window),
                "the entry owns focus on the Enter path",
            );
            field.set_text(&destination.to_string_lossy());
            field.emit_by_name::<()>("activate", &[]);
            wait_until(
                || {
                    destination.join("photo.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "focused Enter moves to the destination",
            );
            assert!(!source.exists(), "Move removes its source");
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn send_to_focused_enter_copies_to_device_root() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_focused_enter_copies_to_device_root",
        || {
            let (_fixture, source_dir, device) =
                focused_enter_destination_fixture("send-to focused-enter fixture");
            let source = source_dir.join("photo.txt");
            let (view, overlay, window, events) = open_transfer_browser(&source_dir);
            let device_id = "volume:focused-enter-device";
            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            assert!(wait_for_modal_layer(&overlay), "Choose folder dialog opens");
            let field = destination_field(&overlay);
            click_button(&overlay, "destination");
            wait_until(
                || destination_entry_owns_focus(&field, &window),
                "the entry owns focus on the Enter path",
            );
            field.set_text("");
            assert!(
                destination_entry_owns_focus(&field, &window),
                "clearing the entry keeps focus for the Enter path"
            );
            field.emit_by_name::<()>("activate", &[]);
            wait_until(
                || {
                    device.join("photo.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "focused Enter copies the empty path to the device root",
            );
            assert!(source.exists(), "sources remain in place");
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(&source_dir)),
                "Send to does not navigate to its destination"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn extract_to_focused_enter_extracts_destination() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::extract_to_focused_enter_extracts_destination",
        || {
            let fixture = tempfile::tempdir().expect("extract focused-enter fixture");
            let work = fixture.path().join("work");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&work).expect("work directory");
            std::fs::create_dir_all(&destination).expect("destination directory");
            std::fs::write(work.join("notes.txt"), b"notes").expect("archived file");
            let archive = work.join("bundle.tar");
            crate::adapters::write_compression_fixture(
                &archive,
                &[work.join("notes.txt")],
                crate::services::ArchiveFormat::Tar,
                None,
            )
            .expect("fixture archive");
            let (view, overlay, window, _events) = open_transfer_browser(&work);
            view.state.show_extract_to_dialog(transfer_entry(&archive));
            assert!(wait_for_modal_layer(&overlay), "Extract dialog opens");
            let field = destination_field(&overlay);
            click_button(&overlay, "work");
            wait_until(
                || destination_entry_owns_focus(&field, &window),
                "the entry owns focus on the Enter path",
            );
            field.set_text(&destination.to_string_lossy());
            field.emit_by_name::<()>("activate", &[]);
            wait_until(
                || {
                    std::fs::read_to_string(destination.join("notes.txt")).is_ok_and(|contents| {
                        contents == "notes"
                            && find_widget_with_class(&overlay, "app-modal-layer").is_none()
                    })
                },
                "focused Enter extracts the archived contents and closes its dialog",
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

fn send_to_toast_labels(overlay: &gtk::Overlay) -> Vec<String> {
    let mut labels = Vec::new();
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("send-to-success") {
            let mut inner = vec![widget.clone()];
            while let Some(node) = inner.pop() {
                if let Some(label) = node.downcast_ref::<gtk::Label>() {
                    labels.push(label.label().to_string());
                    break;
                }
                let mut descendant = node.first_child();
                while let Some(next) = descendant {
                    inner.push(next.clone());
                    descendant = next.next_sibling();
                }
            }
        }
        let mut descendant = widget.first_child();
        while let Some(next) = descendant {
            stack.push(next.clone());
            descendant = next.next_sibling();
        }
    }
    labels
}

struct SendToToastFixture {
    _tempdir: tempfile::TempDir,
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
    events: Rc<RefCell<Vec<crate::app::BrowserEvent>>>,
    source_dir: std::path::PathBuf,
    device: std::path::PathBuf,
}

fn open_send_to_toast_browser(fixture_name: &str, files: &[&str]) -> SendToToastFixture {
    let tempdir = tempfile::tempdir().expect(fixture_name);
    let source_dir = tempdir.path().join("source");
    let device = tempdir.path().join("VANIA");
    std::fs::create_dir_all(&source_dir).expect("source directory");
    std::fs::create_dir_all(&device).expect("device directory");
    for name in files {
        std::fs::write(source_dir.join(name), name.as_bytes()).expect("source file");
    }
    let view = crate::ui::browser::BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.navigate_location(Location::local(&source_dir));
    let overlay = view.overlay();
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    view.browser()
        .observe(move |event| observed.borrow_mut().push(event.clone()));
    SendToToastFixture {
        _tempdir: tempdir,
        view,
        overlay,
        window,
        events,
        source_dir,
        device,
    }
}

#[test]
fn send_to_success_text_formats_single_and_plural() {
    assert_eq!(
        super::ViewState::send_to_success_text("VANIA", 1),
        "Copied to VANIA"
    );
    assert_eq!(
        super::ViewState::send_to_success_text("VANIA", 3),
        "3 items copied to VANIA"
    );
}

#[test]
fn send_to_fast_copy_shows_transient_success() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_fast_copy_shows_transient_success",
        || {
            let SendToToastFixture {
                _tempdir,
                view,
                overlay,
                window,
                events,
                source_dir,
                device,
            } = open_send_to_toast_browser("send-to toast fixture", &["a.txt"]);
            let device_root = device.clone();
            view.state.send_to_removable_device_with_resolver(
                "volume:toast-device",
                vec![Location::local(source_dir.join("a.txt"))],
                move |id| (id == "volume:toast-device").then(|| device_root.clone()),
            );
            wait_until(
                || {
                    device.join("a.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the fast send-to copy",
            );
            assert_eq!(
                send_to_toast_labels(&overlay),
                ["Copied to VANIA"],
                "a fast success shows the transient notice"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn send_to_plural_copy_shows_item_count() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_plural_copy_shows_item_count",
        || {
            let SendToToastFixture {
                _tempdir,
                view,
                overlay,
                window,
                events,
                source_dir,
                device,
            } = open_send_to_toast_browser(
                "send-to plural toast fixture",
                &["a.txt", "b.txt", "c.txt"],
            );
            let device_root = device.clone();
            let sources = ["a.txt", "b.txt", "c.txt"]
                .into_iter()
                .map(|name| Location::local(source_dir.join(name)))
                .collect();
            view.state.send_to_removable_device_with_resolver(
                "volume:toast-device",
                sources,
                move |id| (id == "volume:toast-device").then(|| device_root.clone()),
            );
            wait_until(
                || {
                    ["a.txt", "b.txt", "c.txt"]
                        .iter()
                        .all(|name| device.join(name).exists())
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the plural send-to copy",
            );
            assert_eq!(
                send_to_toast_labels(&overlay),
                ["3 items copied to VANIA"],
                "a plural success counts the items"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn send_to_with_visible_progress_shows_no_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_with_visible_progress_shows_no_toast",
        || {
            let names: Vec<String> = (0..16)
                .map(|index| format!("file-{index:02}.txt"))
                .collect();
            let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
            let SendToToastFixture {
                _tempdir,
                view,
                overlay,
                window,
                events,
                source_dir,
                device,
            } = open_send_to_toast_browser("send-to progress fixture", &borrowed);
            let device_root = device.clone();
            let sources = names
                .iter()
                .map(|name| Location::local(source_dir.join(name)))
                .collect();
            view.state.send_to_removable_device_with_resolver(
                "volume:toast-device",
                sources,
                move |id| (id == "volume:toast-device").then(|| device_root.clone()),
            );
            wait_until(
                || find_widget_with_class(&overlay, "app-modal-layer").is_some(),
                "the progress modal",
            );
            wait_until(
                || {
                    names.iter().all(|name| device.join(name).exists())
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the progress-covered send-to copy",
            );
            wait_until(
                || find_widget_with_class(&overlay, "app-modal-layer").is_none(),
                "the progress modal closes",
            );
            assert!(
                send_to_toast_labels(&overlay).is_empty(),
                "no transient notice follows visible progress"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn normal_copy_fast_shows_no_send_to_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::normal_copy_fast_shows_no_send_to_toast",
        || {
            let fixture = tempfile::tempdir().expect("normal copy toast fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source directory");
            std::fs::create_dir_all(&destination).expect("destination directory");
            let source = source_dir.join("photo.txt");
            std::fs::write(&source, b"photo").expect("source file");
            let (view, overlay, window, events) = open_transfer_browser(&source_dir);
            view.state
                .show_transfer_dialog(vec![transfer_entry(&source)], false);
            assert!(wait_for_modal_layer(&overlay), "Copy dialog opens");
            let field = destination_field(&overlay);
            field.set_text(&destination.to_string_lossy());
            click_button(&overlay, "Copy here");
            wait_until(
                || {
                    destination.join("photo.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the normal fast copy",
            );
            assert!(
                send_to_toast_labels(&overlay).is_empty(),
                "a normal copy never shows send-to feedback"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[cfg(unix)]
#[test]
fn send_to_failure_shows_no_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_failure_shows_no_toast",
        || {
            let SendToToastFixture {
                _tempdir,
                view,
                overlay,
                window,
                events,
                source_dir,
                device,
            } = open_send_to_toast_browser("send-to failure fixture", &["a.txt"]);
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&device, std::fs::Permissions::from_mode(0o555))
                .expect("read-only device");
            let device_root = device.clone();
            view.state.send_to_removable_device_with_resolver(
                "volume:toast-device",
                vec![Location::local(source_dir.join("a.txt"))],
                move |id| (id == "volume:toast-device").then(|| device_root.clone()),
            );
            wait_until(
                || has_visible_button(&overlay, "Unable to complete operation"),
                "the failure dialog",
            );
            assert!(
                send_to_toast_labels(&overlay).is_empty(),
                "a failed send-to shows no success notice"
            );
            assert!(
                events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, crate::app::BrowserEvent::OperationFailed { .. })),
                "the failure was reported"
            );
            assert!(
                events
                    .borrow()
                    .iter()
                    .all(|event| !matches!(event, crate::app::BrowserEvent::TransferCompleted)),
                "no successful completion was reported"
            );
            std::fs::set_permissions(&device, std::fs::Permissions::from_mode(0o755))
                .expect("writable device for cleanup");
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn send_to_repeated_completions_replace_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_repeated_completions_replace_toast",
        || {
            let SendToToastFixture {
                _tempdir,
                view,
                overlay,
                window,
                events,
                source_dir,
                device,
            } = open_send_to_toast_browser(
                "send-to repeated toast fixture",
                &["a.txt", "b.txt", "c.txt"],
            );
            for (id, names) in [
                ("volume:toast-first", vec!["a.txt"]),
                ("volume:toast-second", vec!["b.txt", "c.txt"]),
            ] {
                let device_root = device.clone();
                let sources = names
                    .iter()
                    .map(|name| Location::local(source_dir.join(name)))
                    .collect();
                view.state
                    .send_to_removable_device_with_resolver(id, sources, move |resolved| {
                        (resolved == id).then(|| device_root.clone())
                    });
                wait_until(
                    || {
                        names.iter().all(|name| device.join(name).exists())
                            && events.borrow().iter().any(|event| {
                                matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                            })
                    },
                    "each repeated send-to copy",
                );
                events.borrow_mut().clear();
            }
            assert_eq!(
                send_to_toast_labels(&overlay),
                ["2 items copied to VANIA"],
                "the second success replaces the first notice"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn superseded_send_to_shows_no_success_feedback() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::superseded_send_to_shows_no_success_feedback",
        || {
            let SendToToastFixture {
                _tempdir: _keepalive,
                view,
                overlay,
                window,
                events,
                source_dir,
                device,
            } = open_send_to_toast_browser("superseded send-to fixture", &["a.txt", "b.txt"]);
            std::fs::create_dir_all(device.join("other")).expect("plain destination");
            let device_root = device.clone();
            // A dispatches a Send-to, then B immediately supersedes it with a
            // normal copy before either completes. B's successful completion
            // must not display A's Send-to feedback.
            view.state.send_to_removable_device_with_resolver(
                "volume:toast-device",
                vec![Location::local(source_dir.join("a.txt"))],
                {
                    let device_root = device_root.clone();
                    move |id| (id == "volume:toast-device").then(|| device_root.clone())
                },
            );
            view.state.start_transfer(
                Location::local(device.join("other")),
                vec![Location::local(source_dir.join("b.txt"))],
                false,
            );
            wait_until(
                || {
                    device.join("other/b.txt").exists()
                        && events.borrow().iter().any(|event| {
                            matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                        })
                },
                "the superseding copy completes",
            );
            assert!(
                send_to_toast_labels(&overlay).is_empty(),
                "a superseded Send-to never attributes feedback to the later transfer"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn normal_copy_and_move_to_keep_home_search_creation_and_reveal() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::normal_copy_and_move_to_keep_home_search_creation_and_reveal",
        || {
            let home_destination = glib::home_dir().join("normal-home-search-target");
            std::fs::create_dir_all(&home_destination).expect("home search directory");
            for move_sources in [false, true] {
                let fixture = tempfile::tempdir().expect("normal transfer fixture");
                let source_dir = fixture.path().join("source");
                std::fs::create_dir(&source_dir).expect("source directory");
                let source = source_dir.join("source.txt");
                std::fs::write(&source, b"source").expect("source file");
                let created_destination = fixture.path().join("created-destination");
                let view = crate::ui::browser::BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                view.navigate_location(Location::local(&source_dir));
                let overlay = view.overlay();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                let events = Rc::new(RefCell::new(Vec::new()));
                let observed = events.clone();
                view.browser()
                    .observe(move |event| observed.borrow_mut().push(event.clone()));

                view.state
                    .show_transfer_dialog(vec![transfer_entry(&source)], move_sources);
                assert!(
                    wait_for_modal_layer(&overlay),
                    "normal transfer dialog opens"
                );
                let field = destination_field(&overlay);
                field.set_text("normal-home-search-target");
                wait_until(
                    || {
                        let mut child = find_widget_with_class(&overlay, "transfer-suggestions")
                            .expect("destination suggestions")
                            .first_child();
                        while let Some(widget) = child {
                            child = widget.next_sibling();
                            if widget
                                .downcast_ref::<gtk::Button>()
                                .and_then(|button| button.tooltip_text())
                                .as_deref()
                                == Some(home_destination.to_string_lossy().as_ref())
                            {
                                return true;
                            }
                        }
                        false
                    },
                    "normal text search to find the home destination",
                );
                field.set_text(&created_destination.to_string_lossy());
                let primary_label = if move_sources {
                    "Move here"
                } else {
                    "Copy here"
                };
                let create_label = if move_sources {
                    "Create and move"
                } else {
                    "Create and copy"
                };
                click_button(&overlay, primary_label);
                wait_until(
                    || button_with_label(&overlay, create_label).is_some(),
                    "the existing directory creation confirmation",
                );
                assert!(!created_destination.exists());
                click_button(&overlay, create_label);
                wait_until(
                    || {
                        created_destination.join("source.txt").exists()
                            && events.borrow().iter().any(|event| {
                                matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                            })
                    },
                    "the normal transfer to finish",
                );
                wait_until(
                    || {
                        view.browser().active_location()
                            == Some(Location::local(&created_destination))
                    },
                    "normal completion to navigate to the destination",
                );
                assert_eq!(
                    source.exists(),
                    !move_sources,
                    "Copy leaves its source and Move removes it"
                );
                assert!(
                    events.borrow().iter().any(|event| matches!(
                        event,
                        crate::app::BrowserEvent::TransferReveal { .. }
                    ))
                );
                view.browser().clear_observer();
                window.destroy();
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

#[test]
fn same_folder_paste_creates_a_numbered_copy_without_a_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::same_folder_paste_creates_a_numbered_copy_without_a_dialog",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let folder = fixture.path().join("folder");
            std::fs::create_dir_all(&folder).expect("folder");
            std::fs::write(folder.join("photo.jpg"), b"photo").expect("photo");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&folder),
                vec![Location::local(folder.join("photo.jpg"))],
                false,
            );

            let source = folder.join("photo.jpg");
            let copy = folder.join("photo (1).jpg");
            wait_until(
                || std::fs::read(&copy).is_ok_and(|contents| contents == b"photo"),
                "the numbered duplicate copy",
            );
            assert!(
                find_widget_with_class(&overlay, "app-modal-layer").is_none(),
                "same-folder duplicates must not open the conflict dialog"
            );
            assert_eq!(std::fs::read(&source).expect("original contents"), b"photo");
            window.destroy();
        },
    );
}

#[test]
fn conflict_dialog_offers_skip_for_a_multi_item_paste() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflict_dialog_offers_skip_for_a_multi_item_paste",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source dir");
            std::fs::create_dir_all(&destination).expect("destination dir");
            std::fs::write(source_dir.join("a.txt"), b"new a").expect("source file");
            std::fs::write(source_dir.join("b.txt"), b"new b").expect("source file");
            std::fs::write(destination.join("a.txt"), b"old a").expect("destination file");
            std::fs::write(destination.join("b.txt"), b"old b").expect("destination file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&destination),
                vec![
                    Location::local(source_dir.join("a.txt")),
                    Location::local(source_dir.join("b.txt")),
                ],
                false,
            );

            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            assert!(
                has_visible_button(&overlay, "Skip"),
                "skip must remain for multi-item pastes"
            );
            assert!(
                has_visible_button(&overlay, "Apply to All"),
                "apply to all must appear while further conflicts remain"
            );
            window.destroy();
        },
    );
}

#[test]
fn merging_a_folder_combines_contents_through_the_conflict_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::merging_a_folder_combines_contents_through_the_conflict_dialog",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            let source_folder = source_dir.join("folder");
            std::fs::create_dir_all(&source_folder).expect("source folder");
            std::fs::create_dir_all(destination.join("folder")).expect("destination folder");
            std::fs::write(source_folder.join("incoming.txt"), b"new").expect("source file");
            std::fs::write(source_folder.join("shared.txt"), b"incoming wins")
                .expect("source file");
            std::fs::write(destination.join("folder/stays.txt"), b"keep me")
                .expect("destination file");
            std::fs::write(destination.join("folder/shared.txt"), b"old")
                .expect("destination file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&destination),
                vec![Location::local(&source_folder)],
                false,
            );

            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            assert!(
                has_visible_button(&overlay, "Merge"),
                "a folder collision must offer Merge"
            );

            click_button(&overlay, "Merge");
            let merged = destination.join("folder");
            // The overwritten original is staged in Trash before the copy, so
            // shared.txt is briefly absent: wait for both files' final state.
            wait_until(
                || {
                    std::fs::read(merged.join("incoming.txt"))
                        .is_ok_and(|contents| contents == b"new")
                        && std::fs::read(merged.join("shared.txt"))
                            .is_ok_and(|contents| contents == b"incoming wins")
                },
                "the merge to finish",
            );
            assert_eq!(
                std::fs::read(merged.join("shared.txt")).expect("shared file"),
                b"incoming wins",
                "the incoming item overwrites a same-named destination item"
            );
            assert_eq!(
                std::fs::read(merged.join("stays.txt")).expect("destination-only file"),
                b"keep me",
                "destination-only contents survive the merge"
            );
            assert!(source_folder.is_dir(), "a copy merge keeps the source");
            window.destroy();
        },
    );
}

#[test]
fn merge_is_only_offered_for_copying_a_folder_onto_a_folder() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::merge_is_only_offered_for_copying_a_folder_onto_a_folder",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source dir");
            std::fs::create_dir_all(&destination).expect("destination dir");
            std::fs::write(source_dir.join("file.txt"), b"new").expect("source file");
            std::fs::write(destination.join("file.txt"), b"old").expect("destination file");
            std::fs::create_dir_all(source_dir.join("folder")).expect("source folder");
            std::fs::create_dir_all(destination.join("folder")).expect("destination folder");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&destination),
                vec![Location::local(source_dir.join("file.txt"))],
                false,
            );
            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            assert!(
                !has_visible_button(&overlay, "Merge"),
                "a file collision cannot merge"
            );
            click_button(&overlay, "Cancel");
            wait_until(
                || find_widget_with_class(&overlay, "app-modal-layer").is_none(),
                "the conflict dialog to dismiss",
            );

            view.state.start_transfer(
                Location::local(&destination),
                vec![Location::local(source_dir.join("folder"))],
                true,
            );
            assert!(
                wait_for_modal_layer(&overlay),
                "move conflict dialog modal did not appear"
            );
            assert!(
                !has_visible_button(&overlay, "Merge"),
                "a moved folder collision cannot merge"
            );
            window.destroy();
        },
    );
}

#[test]
fn skipping_the_only_collision_still_transfers_accepted_items() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::skipping_the_only_collision_still_transfers_accepted_items",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source dir");
            std::fs::create_dir_all(&destination).expect("destination dir");
            std::fs::write(source_dir.join("a.txt"), b"new a").expect("source file");
            std::fs::write(source_dir.join("b.txt"), b"new b").expect("source file");
            std::fs::write(source_dir.join("c.txt"), b"new c").expect("source file");
            std::fs::write(source_dir.join("d.txt"), b"new d").expect("source file");
            std::fs::write(destination.join("a.txt"), b"old a").expect("destination file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&destination),
                vec![
                    Location::local(source_dir.join("a.txt")),
                    Location::local(source_dir.join("b.txt")),
                    Location::local(source_dir.join("c.txt")),
                    Location::local(source_dir.join("d.txt")),
                ],
                false,
            );

            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            assert!(
                has_visible_button(&overlay, "Replace"),
                "the single conflicting item must still be resolvable"
            );
            assert!(
                has_visible_button(&overlay, "Skip"),
                "skip must stay visible while other items are already accepted"
            );
            assert!(
                !has_visible_button(&overlay, "Apply to All"),
                "apply to all has nothing left to apply to"
            );

            click_button(&overlay, "Skip");
            for name in ["b.txt", "c.txt", "d.txt"] {
                let copied = destination.join(name);
                let expected = std::fs::read(source_dir.join(name)).expect("source contents");
                wait_until(
                    || std::fs::read(&copied).is_ok_and(|contents| contents == expected),
                    "the non-conflicting transfer to finish",
                );
                assert_eq!(
                    std::fs::read(&copied).expect("copied contents"),
                    expected,
                    "the accepted items must still be pasted"
                );
            }
            assert_eq!(
                std::fs::read(destination.join("a.txt")).expect("colliding file"),
                b"old a",
                "skipping must leave the conflicting file alone"
            );
            assert!(
                !destination.join("a (1).txt").exists(),
                "skipping must not create a numbered copy"
            );
            window.destroy();
        },
    );
}

#[test]
fn skip_stays_visible_for_the_final_conflict_after_keep_both() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::skip_stays_visible_for_the_final_conflict_after_keep_both",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source dir");
            std::fs::create_dir_all(&destination).expect("destination dir");
            std::fs::write(source_dir.join("a.txt"), b"new a").expect("source file");
            std::fs::write(source_dir.join("b.txt"), b"new b").expect("source file");
            std::fs::write(destination.join("a.txt"), b"old a").expect("destination file");
            std::fs::write(destination.join("b.txt"), b"old b").expect("destination file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&destination),
                vec![
                    Location::local(source_dir.join("a.txt")),
                    Location::local(source_dir.join("b.txt")),
                ],
                false,
            );

            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            click_button(&overlay, "Keep Both");

            wait_until(
                || {
                    visible_texts(&overlay).iter().any(|text| text == "b.txt")
                        && !has_visible_button(&overlay, "Apply to All")
                },
                "the final conflict dialog once the first dialog has dismissed",
            );
            assert!(
                has_visible_button(&overlay, "Skip"),
                "skip must stay available for the final conflict after earlier work is accepted"
            );
            assert!(
                !has_visible_button(&overlay, "Apply to All"),
                "apply to all has no further conflicts left to apply to"
            );
            click_button(&overlay, "Skip");

            let kept_both = destination.join("a (1).txt");
            wait_until(
                || std::fs::read(&kept_both).is_ok_and(|contents| contents == b"new a"),
                "the first Keep Both copy",
            );
            assert_eq!(
                std::fs::read(&kept_both).expect("kept-both copy"),
                b"new a",
                "earlier Keep Both choices must be preserved"
            );
            assert_eq!(
                std::fs::read(destination.join("b.txt")).expect("colliding file"),
                b"old b",
                "skipping the final conflict must leave it alone"
            );
            assert!(
                !destination.join("b (1).txt").exists(),
                "skipping must not create a numbered copy"
            );
            window.destroy();
        },
    );
}

#[test]
fn undo_move_keeps_skip_visible_for_a_partial_restore() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::undo_move_keeps_skip_visible_for_a_partial_restore",
        || {
            let fixture = tempfile::tempdir().expect("undo fixture");
            let original = fixture.path().join("original");
            let current = fixture.path().join("current");
            std::fs::create_dir_all(&original).expect("original dir");
            std::fs::create_dir_all(&current).expect("current dir");
            std::fs::write(original.join("a.txt"), b"new a").expect("moved file");
            std::fs::write(original.join("b.txt"), b"new b").expect("moved file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&current),
                vec![
                    Location::local(original.join("a.txt")),
                    Location::local(original.join("b.txt")),
                ],
                true,
            );
            wait_until(
                || current.join("b.txt").exists() && !original.join("b.txt").exists(),
                "the move to complete",
            );
            std::fs::write(original.join("a.txt"), b"blocker").expect("new occupant");

            let browser = view.browser();
            wait_until(
                || browser.pending_undo_move().is_some(),
                "the move undo to become pending",
            );
            let (generation, records) = browser.pending_undo_move().expect("pending move undo");
            assert_eq!(records.len(), 2, "both moved items must be recorded");
            view.state.undo_move(generation, records);

            assert!(
                wait_for_modal_layer(&overlay),
                "undo collision dialog did not appear"
            );
            assert!(
                has_visible_button(&overlay, "Skip"),
                "skip must stay visible when cancelling would discard the accepted restore"
            );
            assert!(
                !has_visible_button(&overlay, "Apply to All"),
                "apply to all has no further conflicts left to apply to"
            );
            click_button(&overlay, "Skip");

            wait_until(
                || !current.join("b.txt").exists(),
                "the accepted item to move back",
            );
            assert_eq!(
                std::fs::read(original.join("b.txt")).expect("restored file"),
                b"new b",
                "the accepted portion of the undo must still be restored"
            );
            assert!(
                current.join("a.txt").exists(),
                "skipping must leave the conflicting move in place"
            );
            assert_eq!(
                std::fs::read(original.join("a.txt")).expect("new occupant"),
                b"blocker",
                "a skipped conflict must not be overwritten"
            );
            window.destroy();
        },
    );
}

#[test]
fn background_move_without_reveal_restores_the_source_column() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::background_move_without_reveal_restores_the_source_column",
        || {
            crate::ui::preferences::PreferenceManager::seed_saved_preferences_for_test();
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_open_folder_after_drop(false);
            let fixture = tempfile::tempdir().expect("drop fixture");
            let source_dir = fixture.path().join("source");
            let child_dir = source_dir.join("child");
            let destination = fixture.path().to_path_buf();
            std::fs::create_dir_all(&child_dir).expect("source directories");
            let source = child_dir.join("file.txt");
            std::fs::write(&source, b"dropped").expect("drop source");
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.set_view_mode(crate::ui::browser_modes::BrowserMode::Columns);
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(
                || {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading)
                },
                "root load",
            );
            assert!(browser.select_entries_by_name_at(0, &[String::from("source")]));
            browser.descend(0, Location::local(&source_dir));
            wait_until(
                || {
                    browser
                        .column_snapshot(1)
                        .is_some_and(|snapshot| !snapshot.loading)
                },
                "source load",
            );
            assert!(browser.select_entries_by_name_at(1, &[String::from("child")]));
            browser.descend(1, Location::local(&child_dir));
            wait_until(
                || {
                    browser
                        .column_snapshot(2)
                        .is_some_and(|snapshot| !snapshot.loading)
                },
                "child load",
            );
            assert_eq!(browser.active_depth(), Some(2));
            let completed = Rc::new(Cell::new(false));
            let observed = completed.clone();
            browser.observe(move |event| {
                if matches!(event, crate::app::BrowserEvent::TransferCompleted) {
                    observed.set(true);
                }
            });
            view.state.drag_source_depth.set(Some(2));
            browser.set_active_column(0);
            view.state.commit_file_drop(
                Location::local(&destination),
                vec![Location::local(&source)],
                DropCommit::Move,
            );
            assert_eq!(view.state.drop_active_depths.get(), Some((2, 0)));

            wait_until(
                || {
                    glib::MainContext::default().iteration(false);
                    completed.get() && browser.active_depth() == Some(2)
                },
                "source focus restoration",
            );
            browser.clear_observer();
        },
    );
}

#[test]
fn redo_move_replays_the_undone_transfer() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::redo_move_replays_the_undone_transfer",
        || {
            let fixture = tempfile::tempdir().expect("redo fixture");
            let original = fixture.path().join("original");
            let current = fixture.path().join("current");
            std::fs::create_dir_all(&original).expect("original dir");
            std::fs::create_dir_all(&current).expect("current dir");
            std::fs::write(original.join("a.txt"), b"moved").expect("moved file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&current),
                vec![Location::local(original.join("a.txt"))],
                true,
            );
            wait_until(
                || current.join("a.txt").exists() && !original.join("a.txt").exists(),
                "the move to complete",
            );

            let browser = view.browser();
            wait_until(
                || browser.pending_undo_move().is_some(),
                "the move undo to become pending",
            );
            let (generation, records) = browser.pending_undo_move().expect("pending move undo");
            assert!(view.state.undo_move(generation, records));
            wait_until(
                || original.join("a.txt").exists() && !current.join("a.txt").exists(),
                "the undo to move the item back",
            );

            wait_until(
                || browser.pending_redo_move().is_some(),
                "the redo of the undone move to become pending",
            );
            let (generation, records) = browser.pending_redo_move().expect("pending move redo");
            assert!(view.state.redo_move(generation, records));
            wait_until(
                || current.join("a.txt").exists() && !original.join("a.txt").exists(),
                "the redo to move the item forward again",
            );
            wait_until(
                || browser.pending_undo_move().is_some(),
                "the completed redo to regenerate the move undo",
            );
            window.destroy();
        },
    );
}

#[test]
fn drop_open_preference_applies_before_settings_and_live_across_views() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::drop_open_preference_applies_before_settings_and_live_across_views",
        || {
            use crate::ui::browser_modes::BrowserMode;
            crate::ui::preferences::PreferenceManager::seed_saved_preferences_for_test();
            let manager = crate::ui::preferences::PreferenceManager::shared();
            assert!(manager.open_folder_after_drop());
            let views: Vec<_> = (0..2)
                .map(|_| {
                    let view = crate::ui::browser::BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        crate::ui::browser::PeekBehavior::default(),
                    );
                    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                    view
                })
                .collect();
            for enabled in [true, false, true] {
                manager.set_open_folder_after_drop(enabled);
                for mode in [BrowserMode::List, BrowserMode::Columns] {
                    for (index, view) in views.iter().enumerate() {
                        view.set_view_mode(mode);
                        let fixture = tempfile::tempdir().expect("drop fixture");
                        let destination = fixture.path().join("destination");
                        std::fs::create_dir(&destination).expect("drop destination");
                        let source = fixture.path().join("file.txt");
                        std::fs::write(&source, b"dropped").expect("drop source");
                        view.navigate_location(Location::local(fixture.path()));
                        let events = Rc::new(RefCell::new(Vec::new()));
                        let observed = events.clone();
                        view.browser()
                            .observe(move |event| observed.borrow_mut().push(event.clone()));
                        let commit = if index == 0 {
                            DropCommit::Copy
                        } else {
                            DropCommit::Move
                        };
                        view.state.commit_file_drop(
                            Location::local(&destination),
                            vec![Location::local(&source)],
                            commit,
                        );
                        wait_until(
                            || {
                                events.borrow().iter().any(|event| {
                                    matches!(
                                        event,
                                        crate::app::BrowserEvent::TransferFinished { .. }
                                    )
                                })
                            },
                            "drop completion",
                        );
                        assert_eq!(
                            std::fs::read(destination.join("file.txt")).expect("dropped file"),
                            b"dropped"
                        );
                        assert_eq!(source.exists(), index == 0);
                        assert_eq!(
                            events.borrow().iter().any(|event| matches!(
                                event,
                                crate::app::BrowserEvent::TransferReveal { .. }
                            )),
                            enabled
                        );
                        let expected = if enabled {
                            Location::local(&destination)
                        } else {
                            Location::local(fixture.path())
                        };
                        assert_eq!(view.browser().active_location(), Some(expected));
                        if enabled && mode == BrowserMode::Columns {
                            assert_eq!(
                                view.browser().location_at(0),
                                Some(Location::local(fixture.path()))
                            );
                            assert_eq!(
                                view.browser().location_at(1),
                                Some(Location::local(&destination))
                            );
                        }
                    }
                }
            }
        },
    );
}

#[test]
fn cross_device_confirmation_reads_the_live_drop_open_preference() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::cross_device_confirmation_reads_the_live_drop_open_preference",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            assert!(!manager.open_folder_after_drop());
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            view.set_view_mode(crate::ui::browser_modes::BrowserMode::List);
            let root = crate::ui::blur::BlurBin::new(&view.widget());
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            for (button, enabled) in [("Copy", true), ("Move", false)] {
                let fixture = tempfile::tempdir().expect("confirmed drop fixture");
                let destination = fixture.path().join("destination");
                std::fs::create_dir(&destination).expect("confirmed destination");
                let source = fixture.path().join("file.txt");
                std::fs::write(&source, b"confirmed").expect("confirmed source");
                view.navigate_location(Location::local(fixture.path()));
                let finished = Rc::new(Cell::new(false));
                let observed = finished.clone();
                view.browser().observe(move |event| {
                    if matches!(event, crate::app::BrowserEvent::TransferFinished { .. }) {
                        observed.set(true);
                    }
                });
                manager.set_open_folder_after_drop(false);
                view.state.commit_file_drop(
                    Location::local(&destination),
                    vec![Location::local(&source)],
                    DropCommit::Ask {
                        default: TransferKind::Copy,
                        volume: VolumeRelation::Different,
                    },
                );
                assert!(wait_for_modal_layer(&overlay));
                click_button(&overlay, "Cancel");
                assert!(source.exists());
                assert!(!destination.join("file.txt").exists());
                assert!(
                    !view.state.suppress_scroll_after_drop.get(),
                    "cancelling confirmation must not suppress subsequent focus and reveal"
                );
                manager.set_open_folder_after_drop(!enabled);
                view.state.commit_file_drop(
                    Location::local(&destination),
                    vec![Location::local(&source)],
                    DropCommit::Ask {
                        default: TransferKind::Copy,
                        volume: VolumeRelation::Different,
                    },
                );
                assert!(wait_for_modal_layer(&overlay));
                manager.set_open_folder_after_drop(enabled);
                click_button(&overlay, button);
                wait_until(|| finished.get(), "confirmed drop completion");
                assert_eq!(
                    std::fs::read(destination.join("file.txt")).expect("confirmed file"),
                    b"confirmed"
                );
                assert_eq!(source.exists(), button == "Copy");
                let expected = if enabled {
                    Location::local(&destination)
                } else {
                    Location::local(fixture.path())
                };
                assert_eq!(view.browser().active_location(), Some(expected));
            }
            window.destroy();
        },
    );
}

#[test]
fn conflict_dialog_verifies_theme_following() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflict_dialog_verifies_theme_following",
        || {
            let themes = crate::ui::theme::ThemeManager::shared();
            themes.select_theme("tokyo-night");
            crate::ui::window::load_styles();

            let fixture = tempfile::tempdir().expect("conflict fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source dir");
            std::fs::create_dir_all(&destination).expect("destination dir");
            std::fs::write(source_dir.join("cast.txt"), b"new").expect("source file");
            std::fs::write(destination.join("cast.txt"), b"old").expect("destination file");

            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let browser_widget = view.widget();
            let root = crate::ui::blur::BlurBin::new(&browser_widget);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&root));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();

            view.state.start_transfer(
                Location::local(&destination),
                vec![Location::local(source_dir.join("cast.txt"))],
                false,
            );
            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );

            let first = resolved_dialog_surface(&overlay);
            themes.select_theme("everforest-light-medium");
            for _ in 0..3 {
                glib::MainContext::default().iteration(false);
            }
            let second = resolved_dialog_surface(&overlay);
            assert_ne!(
                first, second,
                "the conflict dialog must re-theme when the active theme changes"
            );
            window.destroy();
        },
    );
}
