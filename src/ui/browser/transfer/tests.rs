// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{FileEntry, Location};
use crate::services::{DropCommit, RenameCause, TransferKind};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::OsString,
    path::Path,
};

use crate::test_support::operations::operation_event;

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

fn visible_widget<W: IsA<gtk::Widget>>(
    overlay: &gtk::Overlay,
    matches: impl Fn(&W) -> bool,
) -> Option<W> {
    let mut stack = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        stack.push(widget.clone());
        child = widget.next_sibling();
    }
    while let Some(widget) = stack.pop() {
        if let Some(found) = widget.downcast_ref::<W>()
            && found.is_visible()
            && matches(found)
        {
            return Some(found.clone());
        }
        let mut descendant = widget.first_child();
        while let Some(next) = descendant {
            stack.push(next.clone());
            descendant = next.next_sibling();
        }
    }
    None
}

fn button_with_label(overlay: &gtk::Overlay, label: &str) -> Option<gtk::Button> {
    visible_widget(overlay, |button: &gtk::Button| {
        button.label().as_deref() == Some(label)
    })
}

fn check_button_with_label(overlay: &gtk::Overlay, label: &str) -> gtk::CheckButton {
    visible_widget(overlay, |button: &gtk::CheckButton| {
        button.label().as_deref() == Some(label)
    })
    .unwrap_or_else(|| panic!("visible {label:?} check button not found"))
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
        listable: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

fn drain_context() {
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn destination_chooser(parent: &gtk::Window) -> (gtk::Window, crate::ui::browser::BrowserView) {
    wait_until(
        || crate::ui::chooser::destination_chooser_for(parent).is_some(),
        "floating destination chooser",
    );
    crate::ui::chooser::destination_chooser_for(parent).expect("floating destination chooser")
}

fn navigate_destination(view: &crate::ui::browser::BrowserView, path: &Path) {
    view.navigate_location(Location::local(path));
    wait_until(
        || view.browser().active_location() == Some(Location::local(path)),
        "chooser destination navigation",
    );
}

fn destination_accept(window: &gtk::Window) -> gtk::Button {
    window
        .default_widget()
        .expect("chooser default action")
        .downcast::<gtk::Button>()
        .expect("chooser action button")
}

fn confirm_destination(window: &gtk::Window) {
    destination_accept(window).emit_clicked();
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
            && (label.has_css_class("error") || label.has_css_class("form-field-error"))
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
        .lookup_color("strata_surface")
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
        listable: crate::model::MetadataValue::Unknown,
        recent_uri: None,
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
            listable: crate::model::MetadataValue::Unknown,
            ..entry("file.txt")
        };
        assert_eq!(duplicate_transfer(&[trashed]), None);
    }
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
                browser.observe(move |event| match operation_event(event) {
                    crate::app::BrowserEvent::TransferStarted { total, moving } => {
                        observed_started.borrow_mut().push((*total, *moving));
                    }
                    crate::app::BrowserEvent::TransferFinished { .. } => {
                        observed_finished.set(true);
                    }
                    crate::app::BrowserEvent::OperationFailed { message, .. } => {
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
                wait_until(|| !started.borrow().is_empty(), "the transfer to start");
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
            browser
                .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));

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
            browser
                .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));

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
            std::fs::create_dir_all(outside.join("outside-folder")).expect("outside directory");
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
                .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));

            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&first), Location::local(&second)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            let (chooser_window, chooser) = destination_chooser(&window);
            assert_eq!(
                chooser.browser().active_location(),
                Some(Location::local(&device))
            );
            wait_until(
                || {
                    chooser
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading)
                },
                "confined directory listing",
            );
            let listed_inside_device = || {
                let mut position = 0;
                while let Some(entry) = chooser.browser().entry_at(0, position) {
                    assert!(entry.is_directory());
                    assert!(
                        entry
                            .location
                            .native_path()
                            .is_some_and(|path| path.starts_with(&device))
                    );
                    assert_ne!(
                        entry.display_name, "outside-folder",
                        "nothing outside the device is listed"
                    );
                    position += 1;
                }
            };
            listed_inside_device();
            for path in [outside.clone(), device.join("../outside")] {
                chooser.navigate_location(Location::local(&path));
                drain_context();
                assert_eq!(
                    chooser.browser().active_location(),
                    Some(Location::local(&device)),
                    "a path outside the device stays confined"
                );
            }
            let mut invalid = vec![device.join("missing"), device.join("regular-file.txt")];
            #[cfg(unix)]
            invalid.push(device.join("external-link"));
            for path in invalid {
                chooser.navigate_location(Location::local(&path));
                wait_until(
                    || {
                        chooser
                            .browser()
                            .column_snapshot(0)
                            .is_some_and(|snapshot| {
                                snapshot.location == Location::local(&path)
                                    && !snapshot.loading
                                    && snapshot.error.is_some()
                            })
                    },
                    "an invalid folder fails to load",
                );
                listed_inside_device();
                assert!(events.borrow().iter().all(|event| !matches!(
                    event,
                    crate::app::BrowserEvent::TransferStarted { .. }
                )));
                assert_eq!(
                    preferences.send_to_recent_destinations(device_id),
                    original_recents
                );
            }
            #[cfg(unix)]
            {
                let settle = |duration| {
                    let deadline = std::time::Instant::now() + duration;
                    while std::time::Instant::now() < deadline {
                        glib::MainContext::default().iteration(false);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                };
                use crate::ui::browser_modes::BrowserMode;
                let original = chooser.view_mode();
                assert!(chooser.show_filter_with_query("outside"));
                for mode in [BrowserMode::Columns, BrowserMode::List] {
                    chooser.set_view_mode(mode);
                    wait_until(
                        || {
                            chooser
                                .filter_status()
                                .flatten()
                                .is_some_and(|status| status.displayed)
                        },
                        "the search there settles",
                    );
                    settle(std::time::Duration::from_millis(500));
                    assert_eq!(
                        chooser
                            .filter_status()
                            .flatten()
                            .map(|status| status.files + status.folders),
                        Some(0),
                        "{mode:?}: a search there finds nothing outside the device"
                    );
                }
                chooser.dismiss_focused_filter();
                chooser.set_view_mode(original);
            }
            chooser.create_new_folder();
            assert!(
                !chooser.new_entry_is_active(),
                "Send to cannot create folders"
            );
            assert!(!device.join("missing").exists());
            #[cfg(unix)]
            let selected = device.join("internal-link");
            #[cfg(not(unix))]
            let selected = device.join("subdir");
            navigate_destination(&chooser, &selected);
            confirm_destination(&chooser_window);
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
                .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));
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
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &device.join("folder"));
            let chooser_overlay = chooser_window
                .child()
                .expect("chooser content")
                .downcast::<gtk::Overlay>()
                .expect("chooser overlay");
            current.replace(None);
            confirm_destination(&chooser_window);
            wait_until(
                || {
                    visible_error_message(&chooser_overlay).as_deref()
                        == Some("The removable device is no longer available.")
                },
                "the unavailable device error",
            );
            assert!(chooser_window.is_visible(), "the chooser stays open");
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
                .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));
            let current = Rc::new(RefCell::new(Some(opened_root.clone())));
            let resolved = current.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:remounted-device".to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |_| resolved.borrow().clone()),
            );
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &opened_root.join("nested"));
            let chooser_overlay = chooser_window
                .child()
                .expect("chooser content")
                .downcast::<gtk::Overlay>()
                .expect("chooser overlay");
            current.replace(Some(current_root.clone()));
            std::fs::remove_dir_all(&opened_root).expect("old mount removed");
            confirm_destination(&chooser_window);
            wait_until(
                || {
                    visible_texts(&chooser_overlay).iter().any(|text| {
                        text == "Choose an existing folder inside this removable device."
                    }) && destination_accept(&chooser_window).is_sensitive()
                },
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
            confirm_destination(&chooser_window);
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
fn choose_folder_navigation_and_cancellation_leave_parent_unchanged() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder_navigation_and_cancellation_leave_parent_unchanged",
        || {
            let (_fixture, source_dir, device) =
                focused_enter_destination_fixture("chooser navigation fixture");
            let nested = device.join("nested");
            std::fs::create_dir(&nested).expect("nested destination");
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:navigation".into(),
                vec![Location::local(source_dir.join("photo.txt"))],
                Rc::new(move |_| Some(root.clone())),
            );
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &nested);
            chooser.browser().parent();
            wait_until(
                || chooser.browser().active_location() == Some(Location::local(&device)),
                "parent returns to device root",
            );
            chooser.browser().parent();
            drain_context();
            assert_eq!(
                chooser.browser().active_location(),
                Some(Location::local(&device)),
                "Parent cannot escape device root"
            );
            chooser.browser().back();
            wait_until(
                || chooser.browser().active_location() == Some(Location::local(&nested)),
                "Back returns to child",
            );
            chooser.browser().forward();
            wait_until(
                || chooser.browser().active_location() == Some(Location::local(&device)),
                "Forward returns to root",
            );
            chooser_window.close();
            drain_context();
            assert!(!chooser_window.is_visible());
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(&source_dir))
            );
            assert!(
                events.borrow().iter().all(|event| !matches!(
                    event,
                    crate::app::BrowserEvent::TransferStarted { .. }
                ))
            );
            window.destroy();
        },
    );
}

#[test]
fn floating_destination_is_single_flight_and_dies_with_parent() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::floating_destination_is_single_flight_and_dies_with_parent",
        || {
            let (_fixture, source_dir, destination) =
                focused_enter_destination_fixture("chooser lifecycle fixture");
            let source = source_dir.join("photo.txt");
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            let open = || {
                view.state
                    .show_transfer_dialog(vec![transfer_entry(&source)], false)
            };
            open();
            open();
            let (chooser_window, chooser) = destination_chooser(&window);
            open();
            drain_context();
            let windows = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
                .filter(|candidate| {
                    candidate.is_visible() && candidate.transient_for().as_ref() == Some(&window)
                })
                .count();
            assert_eq!(windows, 1, "pending and active requests share one chooser");
            navigate_destination(&chooser, &destination);
            chooser_window.close();
            drain_context();
            open();
            let (reopened, _) = destination_chooser(&window);
            assert_ne!(
                reopened, chooser_window,
                "cancellation releases the per-parent lease"
            );
            window.destroy();
            drain_context();
            assert!(
                !reopened.is_visible(),
                "parent destruction destroys the chooser"
            );
            assert!(!destination.join("photo.txt").exists());
            assert!(
                events.borrow().iter().all(|event| !matches!(
                    event,
                    crate::app::BrowserEvent::TransferStarted { .. }
                ))
            );
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
        .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));
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
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            view.state
                .show_transfer_dialog(vec![transfer_entry(&source)], false);
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &destination);
            let accept = destination_accept(&chooser_window);
            accept.grab_focus();
            accept.emit_by_name::<()>("activate", &[]);
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
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            view.state
                .show_transfer_dialog(vec![transfer_entry(&source)], true);
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &destination);
            let accept = destination_accept(&chooser_window);
            accept.grab_focus();
            accept.emit_by_name::<()>("activate", &[]);
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
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            let device_id = "volume:focused-enter-device";
            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            let (chooser_window, _chooser) = destination_chooser(&window);
            let accept = destination_accept(&chooser_window);
            accept.grab_focus();
            accept.emit_by_name::<()>("activate", &[]);
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
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &destination);
            let accept = destination_accept(&chooser_window);
            accept.grab_focus();
            accept.emit_by_name::<()>("activate", &[]);
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

#[derive(Clone, Copy)]
enum ChosenFor {
    Copy,
    Move,
    Extract,
    SendTo,
}

#[derive(Clone, Copy)]
enum ChosenFolder {
    SlowToAnswer,
    Failing(rustix::io::Errno, &'static str),
    WriteOnly,
}

#[test]
fn a_chosen_folder_is_checked_off_the_ui_thread_while_the_chooser_stays_open() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::a_chosen_folder_is_checked_off_the_ui_thread_while_the_chooser_stays_open",
        || {
            use std::os::unix::fs::PermissionsExt;

            for (case, chosen_for, chosen) in [
                (
                    "copy to a folder slow to answer",
                    ChosenFor::Copy,
                    ChosenFolder::SlowToAnswer,
                ),
                (
                    "extract to a folder slow to answer",
                    ChosenFor::Extract,
                    ChosenFolder::SlowToAnswer,
                ),
                (
                    "send to a device slow to answer",
                    ChosenFor::SendTo,
                    ChosenFolder::SlowToAnswer,
                ),
                (
                    "copy to a folder that is gone",
                    ChosenFor::Copy,
                    ChosenFolder::Failing(rustix::io::Errno::NOENT, "Choose an existing folder."),
                ),
                (
                    "extract to a folder that fails",
                    ChosenFor::Extract,
                    ChosenFolder::Failing(rustix::io::Errno::IO, "Unable to open that location: "),
                ),
                (
                    "move to a write-only folder",
                    ChosenFor::Move,
                    ChosenFolder::WriteOnly,
                ),
                (
                    "extract to a write-only folder",
                    ChosenFor::Extract,
                    ChosenFolder::WriteOnly,
                ),
            ] {
                let (_fixture, work, destination) =
                    focused_enter_destination_fixture("chosen folder fixture");
                let archive = work.join("bundle.tar");
                crate::adapters::write_compression_fixture(
                    &archive,
                    &[work.join("photo.txt")],
                    crate::services::ArchiveFormat::Tar,
                    None,
                )
                .expect("fixture archive");
                let (view, _overlay, window, events) = open_transfer_browser(&work);
                let source = work.join("photo.txt");
                match chosen_for {
                    ChosenFor::Copy | ChosenFor::Move => view.state.show_transfer_dialog(
                        vec![transfer_entry(&source)],
                        matches!(chosen_for, ChosenFor::Move),
                    ),
                    ChosenFor::Extract => {
                        view.state.show_extract_to_dialog(transfer_entry(&archive))
                    }
                    ChosenFor::SendTo => {
                        let root = destination.clone();
                        view.state.show_send_to_folder_dialog_with_resolver(
                            "volume:chosen-device".to_owned(),
                            vec![Location::local(&source)],
                            Rc::new(move |_| Some(root.clone())),
                        );
                    }
                }
                let (chooser_window, chooser) = destination_chooser(&window);
                navigate_destination(&chooser, &destination);
                wait_until(
                    || {
                        chooser
                            .browser()
                            .column_snapshot(0)
                            .is_some_and(|snapshot| !snapshot.loading)
                            && chooser.browser().column_has_monitor(0)
                    },
                    &format!("{case}: the chosen folder's listing and monitor"),
                );
                let write_only = matches!(chosen, ChosenFolder::WriteOnly);
                if write_only {
                    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o300))
                        .expect("write-only destination");
                }
                let chooser_overlay = chooser_window
                    .child()
                    .expect("chooser content")
                    .downcast::<gtk::Overlay>()
                    .expect("chooser overlay");
                let (release, held) = std::sync::mpsc::channel::<()>();
                let checked = std::sync::Arc::new(std::sync::Mutex::new(None));
                let seen = checked.clone();
                let slow = matches!(chosen, ChosenFolder::SlowToAnswer);
                let _scope = crate::adapters::FsQuirks::install(
                    destination.parent().expect("fixture root"),
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::FolderStamp,
                            "destination",
                            0,
                            Box::new(move || {
                                *seen.lock().expect("check thread") =
                                    Some(crate::util::ui_thread::is_current());
                                if slow {
                                    let _released = held.recv();
                                }
                            }),
                        )],
                        fail: match chosen {
                            ChosenFolder::Failing(error, _) => {
                                vec![(crate::adapters::FsOp::FolderStamp, "destination", 0, error)]
                            }
                            ChosenFolder::SlowToAnswer | ChosenFolder::WriteOnly => Vec::new(),
                        },
                        ..crate::adapters::Quirks::ext4()
                    },
                );
                let started = view.browser().last_started_operation();

                confirm_destination(&chooser_window);

                wait_until(
                    || checked.lock().expect("check thread").is_some(),
                    &format!("{case}: the chosen folder to be checked"),
                );
                assert_eq!(
                    *checked.lock().expect("check thread"),
                    Some(false),
                    "{case}: the folder is checked off the UI thread"
                );
                match chosen {
                    ChosenFolder::SlowToAnswer => {
                        for _ in 0..20 {
                            glib::MainContext::default().iteration(false);
                        }
                        assert!(
                            chooser_window.is_visible()
                                && !destination_accept(&chooser_window).is_sensitive(),
                            "{case}: the chooser waits for the folder, open and without a second accept"
                        );
                        assert_eq!(
                            view.browser().last_started_operation(),
                            started,
                            "{case}: nothing starts before the folder answers"
                        );
                        release.send(()).expect("held check");
                        wait_until(
                            || destination.join("photo.txt").exists(),
                            &format!("{case}: the item to land in the chosen folder"),
                        );
                    }
                    ChosenFolder::Failing(_, message) => {
                        wait_until(
                            || {
                                visible_error_message(&chooser_overlay)
                                    .is_some_and(|shown| shown.starts_with(message))
                            },
                            &format!("{case}: the chooser to say what is wrong"),
                        );
                        assert!(
                            chooser_window.is_visible()
                                && destination_accept(&chooser_window).is_sensitive(),
                            "{case}: the chooser stays open for another folder"
                        );
                        assert_eq!(view.browser().last_started_operation(), started);
                        assert!(!destination.join("photo.txt").exists());
                    }
                    ChosenFolder::WriteOnly => {
                        wait_until(
                            || {
                                !chooser_window.is_visible()
                                    && view.browser().last_started_operation() != started
                            },
                            &format!("{case}: the chooser to accept the write-only folder"),
                        );
                        if matches!(chosen_for, ChosenFor::Extract) {
                            wait_until(
                                || {
                                    events.borrow().iter().any(|event| {
                                        matches!(
                                            event,
                                            crate::app::BrowserEvent::OperationFailed { message, .. }
                                                if message.starts_with("Could not open extraction destination")
                                        )
                                    })
                                },
                                &format!(
                                    "{case}: extraction, which lists its destination, to say so"
                                ),
                            );
                        } else {
                            wait_until(
                                || destination.join("photo.txt").exists(),
                                &format!("{case}: the item to land in the write-only folder"),
                            );
                        }
                    }
                }
                if write_only {
                    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o700))
                        .expect("readable destination");
                }
                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn an_extraction_reloads_its_folder_only_without_a_monitor() {
    use crate::services::{DirectoryChange, OperationEvent};

    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::an_extraction_reloads_its_folder_only_without_a_monitor",
        || {
            for (row, watched, active_elsewhere) in [
                ("watched", true, false),
                ("unwatched", false, false),
                ("unwatched, another folder active", false, true),
            ] {
                let fixture = tempfile::tempdir().expect("extraction fixture");
                let archive = fixture.path().join("bundle.tar");
                std::fs::write(&archive, b"archive").expect("archive");
                let other = fixture.path().join("other");
                std::fs::create_dir(&other).expect("other folder");
                let extracted = fixture.path().join("bundle");
                let folder = Location::local(fixture.path());
                let source = Folders::new(if watched {
                    Location::local(&extracted)
                } else {
                    folder.clone()
                });
                let view = crate::ui::browser::BrowserView::new(
                    source.clone(),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let operations =
                    Rc::new(crate::test_support::operations::HeldOperations::default());
                view.browser().set_operation_provider(operations.clone());
                let listed = |depth| {
                    view.browser()
                        .with_column_entries(depth, |entries| {
                            entries
                                .iter()
                                .map(|entry| entry.display_name.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                };
                view.navigate_location(folder.clone());
                wait_until(|| listed(0).len() == 2, "the archive's folder");
                if active_elsewhere {
                    view.browser().descend(0, Location::local(&other));
                    wait_until(
                        || view.browser().location_at(1) == Some(Location::local(&other)),
                        "the other folder",
                    );
                    view.browser().set_active_column(1);
                }
                let loads = (
                    source.loads(&folder),
                    source.loads(&Location::local(&other)),
                );

                view.state.extract_entry(transfer_entry(&archive));
                let id = view
                    .browser()
                    .last_started_operation()
                    .expect("extraction started");
                std::fs::create_dir(&extracted).expect("extracted folder");
                if watched {
                    let notify = source
                        .monitors
                        .borrow()
                        .iter()
                        .find(|(location, _)| *location == folder)
                        .map(|(_, notify)| notify.clone())
                        .expect("the folder's monitor");
                    notify(DirectoryChange::Upsert(FileEntry {
                        kind: crate::model::EntryKind::Directory,
                        ..transfer_entry(&extracted)
                    }));
                }
                operations.emit(
                    id,
                    OperationEvent::Extracted {
                        request_id: id,
                        first_name: Some("bundle".to_owned()),
                        created: vec![Location::local(&extracted)],
                    },
                );

                let selected = || {
                    view.browser()
                        .selected_entries()
                        .into_iter()
                        .map(|entry| entry.location)
                        .collect::<Vec<_>>()
                };
                wait_until(
                    || {
                        listed(0).contains(&"bundle".to_owned())
                            && (active_elsewhere || selected() == [Location::local(&extracted)])
                    },
                    &format!("{row}: the extracted folder to be listed"),
                );
                assert_eq!(
                    (
                        source.loads(&folder) - loads.0,
                        source.loads(&Location::local(&other)) - loads.1
                    ),
                    (usize::from(!watched), 0),
                    "{row}: only the extraction's folder without a monitor reloads"
                );
                view.browser().clear_observer();
            }
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
        if widget.has_css_class("overlay-notice") {
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
    // The toast is for sends that finish before delayed progress appears. Under
    // full-suite I/O load a two-file copy and device flush can outlast the real
    // 350 ms delay, which correctly suppresses the toast and fails these checks.
    // Visible-progress coverage uses an immediate-progress item count instead.
    crate::ui::browser::progress::set_file_progress_delay_for_test(std::time::Duration::from_secs(
        60,
    ));
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
        .observe(move |event| observed.borrow_mut().push(operation_event(event).clone()));
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
fn send_to_checks_the_device_root_off_the_ui_thread_before_copying() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to_checks_the_device_root_off_the_ui_thread_before_copying",
        || {
            let SendToToastFixture {
                _tempdir,
                view,
                window,
                events,
                source_dir,
                device,
                ..
            } = open_send_to_toast_browser("send-to check fixture", &["a.txt"]);
            std::fs::create_dir(device.join("Photos")).expect("recent folder");
            let device_id = "volume:toast-device";
            for (recent, copied) in [
                (None, device.join("a.txt")),
                (Some("Photos"), device.join("Photos/a.txt")),
            ] {
                let parked = crate::test_support::parked::Parked::default();
                let _device = crate::adapters::FsQuirks::install(
                    &device,
                    crate::adapters::Quirks {
                        before: vec![(crate::adapters::FsOp::Access, "*", 0, parked.racer())],
                        ..crate::adapters::Quirks::default()
                    },
                );
                let device_root = device.clone();
                let resolve = move |id: &str| (id == device_id).then(|| device_root.clone());
                let sources = vec![Location::local(source_dir.join("a.txt"))];
                match recent {
                    None => view
                        .state
                        .send_to_removable_device_with_resolver(device_id, sources, resolve),
                    Some(recent) => view.state.send_to_recent_destination_with_resolver(
                        device_id,
                        Path::new(recent),
                        sources,
                        resolve,
                    ),
                }
                wait_until(|| parked.reached(), "the device check");
                assert_eq!(parked.reached_on_ui_thread(), Some(false), "{recent:?}");
                assert!(
                    events.borrow().iter().all(|event| !matches!(
                        event,
                        crate::app::BrowserEvent::TransferStarted { .. }
                    )),
                    "nothing is copied before the device answers: {recent:?}"
                );
                parked.release();
                wait_until(
                    || {
                        copied.exists()
                            && events.borrow().iter().any(|event| {
                                matches!(event, crate::app::BrowserEvent::TransferFinished { .. })
                            })
                    },
                    "the copy once the device answered",
                );
                events.borrow_mut().clear();
            }
            assert_eq!(
                crate::ui::preferences::PreferenceManager::shared()
                    .send_to_recent_destinations(device_id)
                    .first()
                    .map(PathBuf::as_path),
                Some(Path::new("Photos"))
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
            for (locale, expected) in [
                ("en", "3 items copied to VANIA"),
                ("ru", "Скопировано на VANIA: 3 элемента"),
            ] {
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
                rust_i18n::set_locale(locale);
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
                    [expected],
                    "a plural success counts the items in {locale}"
                );
                view.browser().clear_observer();
                window.destroy();
            }
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
                || find_widget_with_class(&overlay, "file-operation-card").is_some(),
                "the progress card",
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
                || {
                    find_widget_with_class(&overlay, "progress-complete")
                        .is_some_and(|widget| widget.is_visible())
                },
                "the completion card",
            );
            find_widget_with_class(&overlay, "progress-complete")
                .and_downcast::<gtk::Button>()
                .expect("completion action")
                .emit_clicked();
            wait_until(
                || find_widget_with_class(&overlay, "file-operation-card").is_none(),
                "the progress card closes",
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
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &destination);
            confirm_destination(&chooser_window);
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
            std::fs::set_permissions(
                source_dir.join("a.txt"),
                std::fs::Permissions::from_mode(0o000),
            )
            .expect("unreadable source");
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
fn overlapping_send_to_preserves_its_destination_and_feedback() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::overlapping_send_to_preserves_its_destination_and_feedback",
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
                        && device.join("a.txt").exists()
                        && send_to_toast_labels(&overlay) == ["Copied to VANIA"]
                },
                "both independent copies complete with the Send-to destination's feedback",
            );
            assert!(source_dir.join("a.txt").exists());
            assert!(source_dir.join("b.txt").exists());
            assert!(!device.join("other/a.txt").exists());
            assert!(
                events.borrow().iter().any(|event| matches!(
                    event,
                    crate::app::BrowserEvent::TransferFinished { .. }
                ))
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn normal_copy_and_move_to_keep_home_navigation_creation_and_reveal() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::normal_copy_and_move_to_keep_home_navigation_creation_and_reveal",
        || {
            let home_destination = glib::home_dir().join("normal-home-search-target");
            std::fs::create_dir_all(&home_destination).expect("home search directory");
            for move_sources in [false, true] {
                let fixture = tempfile::tempdir().expect("normal transfer fixture");
                let source_dir = fixture.path().join("source");
                std::fs::create_dir(&source_dir).expect("source directory");
                let source = source_dir.join("source.txt");
                std::fs::write(&source, b"source").expect("source file");
                let created_destination = source_dir.join("new folder");
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
                view.browser().observe(move |event| {
                    observed.borrow_mut().push(operation_event(event).clone())
                });

                view.state
                    .show_transfer_dialog(vec![transfer_entry(&source)], move_sources);
                let (chooser_window, chooser) = destination_chooser(&window);
                navigate_destination(&chooser, &home_destination);
                navigate_destination(&chooser, &source_dir);
                chooser.create_new_folder();
                wait_until(
                    || created_destination.is_dir(),
                    "chooser creates a destination folder",
                );
                chooser.cancel_rename();
                navigate_destination(&chooser, &created_destination);
                confirm_destination(&chooser_window);
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
fn cancelling_an_operation_keeps_the_live_local_listing_visible() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::cancelling_an_operation_keeps_the_live_local_listing_visible",
        || {
            let fixture = tempfile::tempdir().expect("cancelled transfer fixture");
            std::fs::write(fixture.path().join("copied.txt"), b"copied").expect("copied fixture");
            let root = Location::local(fixture.path());
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.navigate_location(root.clone());
            wait_until(
                || {
                    view.browser()
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 1)
                        && view.browser().column_has_monitor(0)
                },
                "the local listing",
            );
            let request = view.browser().column_request_id(0).expect("loaded request");

            view.state
                .handle(&crate::app::BrowserEvent::OperationCancelled {
                    completed: 1,
                    failed: 0,
                    not_attempted: 1,
                    affected_locations: std::collections::HashSet::from([root]),
                    report: Default::default(),
                });

            assert_eq!(view.browser().column_request_id(0), Some(request));
            let snapshot = view.browser().column_snapshot(0).expect("visible listing");
            assert!(!snapshot.loading);
            assert_eq!(snapshot.count, 1);
        },
    );
}

type Notify = Rc<dyn Fn(crate::services::DirectoryChange)>;

struct Folders {
    unwatched: Location,
    loads: RefCell<Vec<Location>>,
    monitors: RefCell<Vec<(Location, Notify)>>,
}

impl Folders {
    fn new(unwatched: Location) -> Rc<Self> {
        Rc::new(Self {
            unwatched,
            loads: RefCell::default(),
            monitors: RefCell::default(),
        })
    }

    fn loads(&self, location: &Location) -> usize {
        self.loads
            .borrow()
            .iter()
            .filter(|loaded| *loaded == location)
            .count()
    }
}

impl crate::services::FileSource for Folders {
    fn validate_location(
        &self,
        location: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        crate::adapters::LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), crate::services::LocationValidationError>)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.validate_location_async(location, emit)
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        self.loads.borrow_mut().push(request.location.clone());
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn watch(
        &self,
        location: Location,
        _: bool,
        notify: Notify,
    ) -> Option<crate::services::LoadHandle> {
        (location != self.unwatched).then(|| {
            self.monitors.borrow_mut().push((location, notify));
            crate::services::LoadHandle::new(|| {})
        })
    }
}

#[test]
fn a_cancelled_transfer_reloads_its_unwatched_destination_while_a_watched_folder_rescans() {
    use crate::services::{CancelledOperation, DirectoryChange, OperationEvent};

    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::a_cancelled_transfer_reloads_its_unwatched_destination_while_a_watched_folder_rescans",
        || {
            // The view docks a copy in the background and keeps a move in the foreground.
            for moving in [false, true] {
                let row = if moving { "move" } else { "copy" };
                let fixture = tempfile::tempdir().expect("cancelled transfer fixture");
                let item = fixture.path().join("item.txt");
                std::fs::write(&item, b"item").expect("transferred item");
                let destination_path = fixture.path().join("destination");
                std::fs::create_dir(&destination_path).expect("destination folder");
                let target = destination_path.join("item.txt");
                let root = Location::local(fixture.path());
                let destination = Location::local(&destination_path);
                let source = Folders::new(destination.clone());
                let view = crate::ui::browser::BrowserView::new(
                    source.clone(),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let operations =
                    Rc::new(crate::test_support::operations::HeldOperations::default());
                view.browser().set_operation_provider(operations.clone());
                let names = |depth| {
                    let mut names = view
                        .browser()
                        .with_column_entries(depth, |entries| {
                            entries
                                .iter()
                                .map(|entry| entry.display_name.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    names.sort();
                    names
                };
                let loads = |location: &Location| source.loads(location);
                view.navigate_location(root.clone());
                wait_until(|| names(0).len() == 2, "the watched folder's listing");
                view.browser().descend(0, destination.clone());
                wait_until(
                    || {
                        view.browser()
                            .column_snapshot(1)
                            .is_some_and(|snapshot| !snapshot.loading)
                    },
                    "the destination's listing",
                );
                let loaded = (loads(&root), loads(&destination));

                view.browser().transfer(
                    destination.clone(),
                    vec![PasteItem {
                        source: Location::local(&item),
                        conflict: TransferConflict::FailIfExists,
                        target_name: None,
                    }],
                    moving,
                    false,
                );
                let id = view
                    .browser()
                    .last_started_operation()
                    .expect("transfer started");
                assert_eq!(view.browser().has_foreground_operation(), moving, "{row}");
                if moving {
                    std::fs::rename(&item, &target).expect("moved item");
                } else {
                    std::fs::copy(&item, &target).expect("copied item");
                }
                let rescan = source
                    .monitors
                    .borrow()
                    .iter()
                    .find(|(watched, _)| *watched == root)
                    .map(|(_, notify)| notify.clone())
                    .expect("the watched folder's monitor");
                rescan(DirectoryChange::Rescan);
                view.browser().cancel_operation(id);
                operations.emit(
                    id,
                    OperationEvent::Cancelled {
                        request_id: id,
                        result: CancelledOperation {
                            completed: vec![Location::local(&item)],
                            failed: Vec::new(),
                            not_attempted: Vec::new(),
                            affected_locations: HashSet::from([
                                root.clone(),
                                destination.clone(),
                                Location::local(&item),
                                Location::local(&target),
                            ]),
                        },
                    },
                );

                wait_until(
                    || names(1) == ["item.txt"],
                    &format!("{row}: the destination to list what the transfer placed"),
                );
                let listed = if moving {
                    vec!["destination"]
                } else {
                    vec!["destination", "item.txt"]
                };
                wait_until(
                    || names(0) == listed,
                    &format!("{row}: the watched folder's rescan"),
                );
                assert_eq!(
                    (loads(&root), loads(&destination)),
                    (loaded.0 + 1, loaded.1 + 1),
                    "{row}: each folder reloads once"
                );
                view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn a_completed_transfer_reloads_its_unwatched_destination_once() {
    use crate::services::OperationEvent;

    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::a_completed_transfer_reloads_its_unwatched_destination_once",
        || {
            // The view docks a copy in the background and keeps a move in the foreground.
            for moving in [false, true] {
                let row = if moving { "move" } else { "copy" };
                let fixture = tempfile::tempdir().expect("completed transfer fixture");
                let item = fixture.path().join("item.txt");
                std::fs::write(&item, b"new item").expect("transferred item");
                let destination_path = fixture.path().join("destination");
                std::fs::create_dir(&destination_path).expect("destination folder");
                let target = destination_path.join("item.txt");
                std::fs::write(&target, b"old").expect("replaced item");
                let root = Location::local(fixture.path());
                let destination = Location::local(&destination_path);
                let source = Folders::new(destination.clone());
                let view = crate::ui::browser::BrowserView::new(
                    source.clone(),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let operations =
                    Rc::new(crate::test_support::operations::HeldOperations::default());
                view.browser().set_operation_provider(operations.clone());
                let loaded = |depth| {
                    view.browser()
                        .column_snapshot(depth)
                        .is_some_and(|snapshot| !snapshot.loading && snapshot.count > 0)
                };
                view.navigate_location(root.clone());
                wait_until(|| loaded(0), "the watched folder's listing");
                view.browser().descend(0, destination.clone());
                wait_until(|| loaded(1), "the destination's listing");
                let loads = source.loads(&destination);

                view.browser().transfer(
                    destination.clone(),
                    vec![PasteItem {
                        source: Location::local(&item),
                        conflict: TransferConflict::ReplaceExisting,
                        target_name: None,
                    }],
                    moving,
                    true,
                );
                let id = view
                    .browser()
                    .last_started_operation()
                    .expect("transfer started");
                assert_eq!(view.browser().has_foreground_operation(), moving, "{row}");
                if moving {
                    std::fs::rename(&item, &target).expect("moved item");
                } else {
                    std::fs::copy(&item, &target).expect("copied item");
                }
                operations.emit(
                    id,
                    OperationEvent::TransferProgress {
                        request_id: id,
                        completed_items: 1,
                        completed_files: 1,
                        total_files: Some(1),
                        current_file: None,
                        transferred_bytes: 8,
                        total_bytes: Some(8),
                        created_location: (!moving).then(|| Location::local(&target)),
                    },
                );
                operations.emit(
                    id,
                    OperationEvent::Pasted {
                        request_id: id,
                        locations: vec![Location::local(&item)],
                    },
                );

                wait_until(
                    || {
                        loaded(1)
                            && view
                                .browser()
                                .selected_entries()
                                .into_iter()
                                .map(|entry| entry.location)
                                .eq([Location::local(&target)])
                    },
                    &format!("{row}: the destination to reveal the replaced item"),
                );
                assert_eq!(
                    source.loads(&destination),
                    loads + 1,
                    "{row}: the destination reloads once"
                );
                view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn transfer_reveal_waits_for_the_live_monitor_instead_of_reloading() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::transfer_reveal_waits_for_the_live_monitor_instead_of_reloading",
        || {
            let fixture = tempfile::tempdir().expect("transfer reveal fixture");
            let destination = Location::local(fixture.path());
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.navigate_location(destination.clone());
            wait_until(
                || {
                    view.browser()
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 0)
                        && view.browser().column_has_monitor(0)
                },
                "the empty local destination",
            );
            let request = view.browser().column_request_id(0).expect("loaded request");
            let first = fixture.path().join("first.txt");
            let second = fixture.path().join("second.txt");
            let selected_locations = || {
                view.browser()
                    .selected_entries()
                    .into_iter()
                    .map(|entry| entry.location)
                    .collect::<Vec<_>>()
            };
            view.state
                .handle(&crate::app::BrowserEvent::TransferReveal {
                    destination,
                    locations: vec![Location::local(&first), Location::local(&second)],
                });

            std::fs::write(&first, b"first").expect("first copied fixture");
            wait_until(
                || selected_locations() == [Location::local(&first)],
                "the first monitor update to select its target",
            );
            assert!(view.state.pending_location_selection.borrow().is_some());
            std::fs::write(&second, b"second").expect("second copied fixture");
            wait_until(
                || selected_locations() == [Location::local(&first), Location::local(&second)],
                "both transferred targets to be selected",
            );
            assert!(view.state.pending_location_selection.borrow().is_none());
            assert_eq!(view.browser().column_request_id(0), Some(request));
        },
    );
}

#[test]
fn revealing_an_already_open_local_destination_keeps_its_live_listing() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::revealing_an_already_open_local_destination_keeps_its_live_listing",
        || {
            let fixture = tempfile::tempdir().expect("transfer reveal fixture");
            let copied = fixture.path().join("copied.txt");
            std::fs::write(&copied, b"copied").expect("copied fixture");
            let destination = Location::local(fixture.path());
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.navigate_location(destination.clone());
            wait_until(
                || {
                    view.browser()
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading && snapshot.count == 1)
                        && view.browser().column_has_monitor(0)
                },
                "the local destination listing",
            );
            let request = view.browser().column_request_id(0).expect("loaded request");

            view.state
                .handle(&crate::app::BrowserEvent::TransferReveal {
                    destination,
                    locations: vec![Location::local(&copied)],
                });

            assert_eq!(view.browser().column_request_id(0), Some(request));
            assert_eq!(
                view.browser()
                    .selected_entries()
                    .into_iter()
                    .map(|entry| entry.location)
                    .collect::<Vec<_>>(),
                [Location::local(copied)]
            );
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
                if matches!(
                    operation_event(event),
                    crate::app::BrowserEvent::TransferCompleted
                ) {
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
            wait_until(
                || view.state.drop_active_depths.get().is_some(),
                "the move to start",
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

fn moved_for_undo(
    view: &crate::ui::browser::BrowserView,
    root: &Path,
    name: &str,
    make: impl FnOnce(&Path),
) -> (std::path::PathBuf, std::path::PathBuf) {
    let (original, current) = (root.join("original"), root.join("current"));
    std::fs::create_dir_all(&original).expect("original dir");
    std::fs::create_dir_all(&current).expect("current dir");
    let (original, current) = (original.join(name), current.join(name));
    make(&original);
    view.state.start_transfer(
        Location::local(current.parent().expect("current dir")),
        vec![Location::local(&original)],
        true,
    );
    wait_until(
        || current.symlink_metadata().is_ok() && original.symlink_metadata().is_err(),
        "the move to complete",
    );
    wait_until(
        || view.browser().pending_undo_move().is_some(),
        "the move undo to become pending",
    );
    (original, current)
}

#[test]
fn an_undo_checks_its_items_off_the_ui_thread_and_a_cancelled_check_keeps_it() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::an_undo_checks_its_items_off_the_ui_thread_and_a_cancelled_check_keeps_it",
        || {
            for cancelled in [false, true] {
                let row = format!("cancelled {cancelled}");
                let fixture = tempfile::tempdir().expect("undo fixture");
                let (view, overlay, window) = conflict_view();
                let (original, current) =
                    moved_for_undo(&view, fixture.path(), "notes.txt", |path| {
                        std::fs::write(path, b"notes").expect("moved file");
                    });
                let browser = view.browser();
                let (release, held) = std::sync::mpsc::channel::<()>();
                let checked = std::sync::Arc::new(std::sync::Mutex::new(None));
                let seen = checked.clone();
                let _scope = crate::adapters::FsQuirks::install(
                    fixture.path(),
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::Stat,
                            "notes.txt",
                            0,
                            Box::new(move || {
                                *seen.lock().expect("check thread") =
                                    Some(crate::util::ui_thread::is_current());
                                let _released = held.recv();
                            }),
                        )],
                        ..crate::adapters::Quirks::ext4()
                    },
                );
                let started = browser.last_started_operation();
                let (generation, records) = browser.pending_undo_move().expect("pending undo");

                assert!(view.state.undo_move(generation, records.clone()));
                wait_until(
                    || checked.lock().expect("check thread").is_some(),
                    &format!("{row}: the undo to check its items"),
                );
                assert_eq!(
                    *checked.lock().expect("check thread"),
                    Some(false),
                    "{row}: the items are checked off the UI thread"
                );
                assert!(
                    view.state.undo_move(generation, records),
                    "{row}: a repeated undo is taken while the first one checks"
                );
                wait_until(
                    || {
                        transfer_row_shown(&overlay, true)
                            && visible_texts(&overlay)
                                .iter()
                                .any(|text| text == "Preparing items…")
                    },
                    &format!("{row}: the undo to show it prepares its items"),
                );
                assert_eq!(
                    browser.last_started_operation(),
                    started,
                    "{row}: nothing moves while the items are checked"
                );
                if cancelled {
                    click_button(&overlay, "Cancel");
                    wait_until(
                        || !transfer_row_shown(&overlay, true),
                        &format!("{row}: the preparing undo to go away"),
                    );
                    release.send(()).expect("held check");
                    for _ in 0..20 {
                        glib::MainContext::default().iteration(false);
                    }
                    assert!(
                        current.exists() && !original.exists(),
                        "{row}: a cancelled undo moves nothing"
                    );
                    let (generation, records) = browser
                        .pending_undo_move()
                        .expect("a cancelled undo stays in the history");
                    assert!(view.state.undo_move(generation, records));
                } else {
                    release.send(()).expect("held check");
                }
                wait_until(
                    || original.exists() && !current.exists(),
                    &format!("{row}: the undo to move the item back"),
                );
                window.destroy();
            }
        },
    );
}

#[test]
fn an_undo_tries_items_it_cannot_rule_out_and_drops_only_items_proven_gone() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::an_undo_tries_items_it_cannot_rule_out_and_drops_only_items_proven_gone",
        || {
            for (case, make, failed_check, undone) in [
                (
                    "a dangling link",
                    (|path: &Path| {
                        std::os::unix::fs::symlink("nowhere", path).expect("dangling link")
                    }) as fn(&Path),
                    None,
                    true,
                ),
                (
                    "a check of the item that fails",
                    |path: &Path| std::fs::write(path, b"notes").expect("moved file"),
                    Some(0),
                    true,
                ),
                (
                    "a check of its destination that fails",
                    |path: &Path| std::fs::write(path, b"notes").expect("moved file"),
                    Some(1),
                    true,
                ),
                (
                    "an item deleted since",
                    |path: &Path| std::fs::write(path, b"notes").expect("moved file"),
                    None,
                    false,
                ),
            ] {
                let fixture = tempfile::tempdir().expect("undo fixture");
                let (view, overlay, window) = conflict_view();
                let (original, current) = moved_for_undo(&view, fixture.path(), "notes.txt", make);
                if !undone {
                    std::fs::remove_file(&current).expect("item deleted since the move");
                }
                let checked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let seen = checked.clone();
                let _scope = crate::adapters::FsQuirks::install(
                    fixture.path(),
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::Stat,
                            "notes.txt",
                            0,
                            Box::new(move || seen.store(true, std::sync::atomic::Ordering::SeqCst)),
                        )],
                        fail: failed_check
                            .map(|skipped| {
                                (
                                    crate::adapters::FsOp::Stat,
                                    "notes.txt",
                                    skipped,
                                    rustix::io::Errno::IO,
                                )
                            })
                            .into_iter()
                            .collect(),
                        ..crate::adapters::Quirks::ext4()
                    },
                );
                let browser = view.browser();
                let started = browser.last_started_operation();
                let (generation, records) = browser.pending_undo_move().expect("pending undo");

                assert!(view.state.undo_move(generation, records));

                wait_until(
                    || checked.load(std::sync::atomic::Ordering::SeqCst),
                    &format!("{case}: the undo to check the item"),
                );
                if undone {
                    wait_until(
                        || {
                            original.symlink_metadata().is_ok()
                                && current.symlink_metadata().is_err()
                        },
                        &format!("{case}: the undo to move the item back"),
                    );
                    assert!(
                        !visible_texts(&overlay)
                            .iter()
                            .any(|text| text == "File already exists"),
                        "{case}: a destination that cannot be ruled out is not replaced"
                    );
                } else {
                    wait_until(
                        || browser.pending_undo_move().is_none(),
                        &format!("{case}: the undo to be dropped"),
                    );
                    assert_eq!(
                        browser.last_started_operation(),
                        started,
                        "{case}: nothing to move back"
                    );
                }
                window.destroy();
            }
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
                        view.browser().observe(move |event| {
                            observed.borrow_mut().push(operation_event(event).clone())
                        });
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
                        wait_until(
                            || view.browser().active_location() == Some(expected.clone()),
                            "the revealed folder",
                        );
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
                    if matches!(
                        operation_event(event),
                        crate::app::BrowserEvent::TransferFinished { .. }
                    ) {
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

#[test]
fn a_conflict_prompt_says_which_name_the_item_takes_and_what_a_merge_overwrites() {
    let destination = Location::local("/media/stick");
    let explanation = |source: &str, target: &str, fixes_inside, merge| {
        conflict_explanation(
            &TransferCollision {
                source: Location::local(format!("/home/user/{source}")),
                target_name: Some(ShownName {
                    name: OsString::from(target),
                    cause: (source != target).then_some(RenameCause::Invalid),
                }),
                mergeable: true,
                fixes_inside,
            },
            &destination,
            merge,
        )
    };
    let saved_as = rust_i18n::t!(
        "“%{name}” will be saved as “%{new_name}” in this location.",
        name = "a:b",
        new_name = "a_b"
    );
    for merge in [false, true] {
        let as_named = explanation("a_b", "a_b", false, merge);
        assert!(!as_named.contains("a:b"), "{as_named}");
        assert_eq!(
            explanation("a:b", "a_b", false, merge),
            crate::i18n::join_sentences(&saved_as, &as_named)
        );
    }
    let replacing = explanation("folder", "folder", true, false);
    assert_eq!(replacing, explanation("folder", "folder", false, false));
    let merging = explanation("folder", "folder", false, true);
    assert_eq!(
        explanation("folder", "folder", true, true),
        crate::i18n::join_sentences(
            &merging,
            &crate::i18n::tr(
                "Items whose names change to suit this location overwrite items with the new name."
            )
        )
    );
}

fn conflict_view() -> (crate::ui::browser::BrowserView, gtk::Overlay, gtk::Window) {
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
    (view, overlay, window)
}

struct FatPaste {
    sources: &'static [&'static str],
    files: &'static [(&'static str, &'static str)],
    disk: &'static [(&'static str, &'static str)],
    prompt: (&'static str, &'static str),
    choice: (&'static str, bool),
    holds: &'static [(&'static str, &'static str)],
}

#[test]
fn a_paste_onto_fat_asks_about_fixed_names_and_uses_the_names_it_planned() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::a_paste_onto_fat_asks_about_fixed_names_and_uses_the_names_it_planned",
        || {
            let rows = [
                FatPaste {
                    sources: &["a:b"],
                    files: &[("a:b", "incoming")],
                    disk: &[("a_b", "old")],
                    prompt: ("a:b", "a_b"),
                    choice: ("Replace", false),
                    holds: &[("a_b", "incoming")],
                },
                FatPaste {
                    sources: &["a:b", "c:d"],
                    files: &[("a:b", "first"), ("c:d", "second")],
                    disk: &[("a_b", "old"), ("c_d", "old")],
                    prompt: ("a:b", "a_b"),
                    choice: ("Replace", true),
                    holds: &[("a_b", "first"), ("c_d", "second")],
                },
                FatPaste {
                    sources: &["a:b", "a?b"],
                    files: &[("a:b", "first"), ("a?b", "second")],
                    disk: &[("a_b", "old")],
                    prompt: ("a:b", "a_b"),
                    choice: ("Skip", false),
                    holds: &[("a_b", "old"), ("a_b (1)", "second")],
                },
                FatPaste {
                    sources: &["x:y"],
                    files: &[("x:y/new.txt", "new")],
                    disk: &[("x_y/old.txt", "old")],
                    prompt: ("x:y", "x_y"),
                    choice: ("Merge", false),
                    holds: &[("x_y/new.txt", "new"), ("x_y/old.txt", "old")],
                },
            ];
            for row in rows {
                let fixture = tempfile::tempdir().expect("conflict fixture");
                let source = fixture.path().join("source");
                let disk = fixture.path().join("disk");
                for (root, files) in [(&source, row.files), (&disk, row.disk)] {
                    for (path, contents) in files {
                        let path = root.join(path);
                        std::fs::create_dir_all(path.parent().expect("parent"))
                            .expect("fixture folder");
                        std::fs::write(path, contents).expect("fixture file");
                    }
                }
                let _disk =
                    crate::adapters::FsQuirks::install(&disk, crate::adapters::Quirks::vfat());
                let (view, overlay, window) = conflict_view();

                view.state.start_transfer(
                    Location::local(&disk),
                    row.sources
                        .iter()
                        .map(|name| Location::local(source.join(name)))
                        .collect(),
                    false,
                );

                assert!(
                    wait_for_modal_layer(&overlay),
                    "conflict dialog modal did not appear"
                );
                let (name, new_name) = row.prompt;
                assert!(
                    visible_texts(&overlay)
                        .iter()
                        .any(|text| text.contains(name) && text.contains(new_name)),
                    "the prompt names the item and the name it takes: {:?}",
                    visible_texts(&overlay)
                );
                let (choice, apply_to_all) = row.choice;
                if apply_to_all {
                    check_button_with_label(&overlay, "Apply to All").set_active(true);
                }
                click_button(&overlay, choice);
                wait_until(
                    || {
                        row.holds.iter().all(|(path, contents)| {
                            std::fs::read(disk.join(path))
                                .is_ok_and(|held| held == contents.as_bytes())
                        })
                    },
                    &format!("{choice} to give each item the name its prompt planned"),
                );
                let mut listed = std::fs::read_dir(&disk)
                    .expect("disk listing")
                    .map(|entry| entry.expect("disk entry").file_name())
                    .collect::<Vec<_>>();
                listed.sort();
                let mut expected = row
                    .holds
                    .iter()
                    .map(|(path, _)| path.split('/').next().expect("top-level name").into())
                    .collect::<Vec<std::ffi::OsString>>();
                expected.dedup();
                assert_eq!(listed, expected, "{choice}: nothing else lands beside them");
                window.destroy();
            }
        },
    );
}

#[test]
fn keeping_both_on_a_move_puts_the_item_beside_the_other_and_undo_moves_it_back() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::keeping_both_on_a_move_puts_the_item_beside_the_other_and_undo_moves_it_back",
        || {
            let fixture = tempfile::tempdir().expect("move fixture");
            let original = fixture.path().join("original");
            let current = fixture.path().join("current");
            std::fs::create_dir_all(&original).expect("original dir");
            std::fs::create_dir_all(&current).expect("current dir");
            std::fs::write(original.join("todo.txt"), b"moved").expect("moved file");
            std::fs::write(current.join("todo.txt"), b"existing").expect("existing file");
            let (view, overlay, window) = conflict_view();

            view.state.start_transfer(
                Location::local(&current),
                vec![Location::local(original.join("todo.txt"))],
                true,
            );
            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            click_button(&overlay, "Keep Both");
            let kept = current.join("todo (1).txt");
            wait_until(
                || {
                    std::fs::read(&kept).is_ok_and(|contents| contents == b"moved")
                        && !original.join("todo.txt").exists()
                },
                "the move beside the existing item",
            );

            let browser = view.browser();
            wait_until(
                || browser.pending_undo_move().is_some(),
                "the move undo to become pending",
            );
            let (generation, records) = browser.pending_undo_move().expect("pending move undo");
            assert!(view.state.undo_move(generation, records));
            wait_until(
                || {
                    std::fs::read(original.join("todo.txt"))
                        .is_ok_and(|contents| contents == b"moved")
                        && !kept.exists()
                },
                "the undo to move the item back",
            );
            assert_eq!(
                std::fs::read(current.join("todo.txt")).expect("existing file"),
                b"existing"
            );
            window.destroy();
        },
    );
}

fn transfer_row_shown(overlay: &gtk::Overlay, moving: bool) -> bool {
    if moving {
        visible_widget(overlay, |layer: &gtk::Box| {
            layer.has_css_class("app-modal-layer") && !layer.has_css_class("dismissing")
        })
        .is_some()
    } else {
        find_widget_with_class(overlay, "app-modal-layer").is_none()
            && visible_widget(overlay, |card: &gtk::Box| {
                card.has_css_class("file-operation-card")
            })
            .is_some()
    }
}

#[test]
fn closing_the_view_while_a_transfer_prepares_dismisses_its_row_and_copies_nothing() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::closing_the_view_while_a_transfer_prepares_dismisses_its_row_and_copies_nothing",
        || {
            for moving in [false, true] {
                let fixture = tempfile::tempdir().expect("slow fixture");
                let source = fixture.path().join("source/notes.txt");
                let disk = fixture.path().join("disk");
                std::fs::create_dir_all(&disk).expect("disk dir");
                std::fs::create_dir_all(fixture.path().join("source")).expect("source dir");
                std::fs::write(&source, b"notes").expect("source file");
                let (release, held) = std::sync::mpsc::channel::<()>();
                let answered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let looked_up = answered.clone();
                let _disk = crate::adapters::FsQuirks::install(
                    &disk,
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::Stat,
                            "notes.txt",
                            0,
                            Box::new(move || {
                                let _released = held.recv();
                                looked_up.store(true, std::sync::atomic::Ordering::SeqCst);
                            }),
                        )],
                        ..crate::adapters::Quirks::ext4()
                    },
                );
                let (view, overlay, window) = conflict_view();
                view.state.start_transfer(
                    Location::local(&disk),
                    vec![Location::local(&source)],
                    moving,
                );
                wait_until(
                    || transfer_row_shown(&overlay, moving),
                    &format!("moving {moving}: the transfer to show it prepares its items"),
                );

                view.dispose_file_progress();

                assert!(
                    !transfer_row_shown(&overlay, moving),
                    "moving {moving}: a closed view keeps showing its preparing transfer"
                );
                release.send(()).expect("held listing");
                wait_until(
                    || answered.load(std::sync::atomic::Ordering::SeqCst),
                    &format!("moving {moving}: the destination to answer"),
                );
                for _ in 0..20 {
                    glib::MainContext::default().iteration(false);
                }
                assert!(
                    !disk.join("notes.txt").exists() && source.exists(),
                    "moving {moving}: a transfer whose view closed while it prepared went on"
                );
                window.destroy();
            }
        },
    );
}

#[test]
fn a_destination_slow_to_answer_shows_the_transfer_preparing_where_its_row_will_be() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::a_destination_slow_to_answer_shows_the_transfer_preparing_where_its_row_will_be",
        || {
            for (moving, cancelled) in [(false, true), (true, true), (false, false), (true, false)]
            {
                let row = format!("moving {moving}, cancelled {cancelled}");
                let fixture = tempfile::tempdir().expect("slow fixture");
                let source = fixture.path().join("source/notes.txt");
                let disk = fixture.path().join("disk");
                std::fs::create_dir_all(&disk).expect("disk dir");
                std::fs::create_dir_all(fixture.path().join("source")).expect("source dir");
                std::fs::write(&source, b"notes").expect("source file");
                let (release, held) = std::sync::mpsc::channel::<()>();
                let answered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let looked_up = answered.clone();
                let _disk = crate::adapters::FsQuirks::install(
                    &disk,
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::Stat,
                            "notes.txt",
                            0,
                            Box::new(move || {
                                let _released = held.recv();
                                looked_up.store(true, std::sync::atomic::Ordering::SeqCst);
                            }),
                        )],
                        ..crate::adapters::Quirks::ext4()
                    },
                );
                let (view, overlay, window) = conflict_view();
                let started = Rc::new(RefCell::new(Vec::new()));
                let observed = started.clone();
                let shown_on = overlay.clone();
                view.browser().observe(move |event| {
                    if matches!(
                        operation_event(event),
                        crate::app::BrowserEvent::TransferStarted { .. }
                    ) {
                        observed
                            .borrow_mut()
                            .push(transfer_row_shown(&shown_on, moving));
                    }
                });
                let paste = || {
                    view.state.start_transfer(
                        Location::local(&disk),
                        vec![Location::local(&source)],
                        moving,
                    )
                };

                paste();
                wait_until(
                    || transfer_row_shown(&overlay, moving),
                    &format!("{row}: the transfer to show it prepares its items"),
                );
                assert!(
                    visible_texts(&overlay).contains(&"notes.txt".to_owned()) || moving,
                    "{row}: the docked row names the item"
                );
                let shown = visible_texts(&overlay);
                assert!(
                    shown.iter().any(|text| text == "Preparing items…")
                        && !shown.iter().any(|text| text == "0/0"),
                    "{row}: the transfer says it prepares its items, with no count: {shown:?}"
                );
                paste();
                if cancelled {
                    if moving {
                        click_button(&overlay, "Cancel");
                    } else {
                        visible_widget(&overlay, |button: &gtk::Button| {
                            button.has_css_class("progress-cancel")
                        })
                        .expect("the docked row's cancel")
                        .emit_clicked();
                    }
                    wait_until(
                        || !transfer_row_shown(&overlay, moving),
                        &format!("{row}: the preparing transfer to go away"),
                    );
                    paste();
                } else {
                    release.send(()).expect("held listing");
                }
                wait_until(
                    || {
                        std::fs::read(disk.join("notes.txt"))
                            .is_ok_and(|contents| contents == b"notes")
                            && source.exists() != moving
                    },
                    &format!("{row}: the transfer"),
                );
                if cancelled {
                    release.send(()).expect("held listing");
                }
                wait_until(
                    || answered.load(std::sync::atomic::Ordering::SeqCst),
                    &format!("{row}: the destination to answer"),
                );
                for _ in 0..20 {
                    glib::MainContext::default().iteration(false);
                }
                assert_eq!(
                    *started.borrow(),
                    [!cancelled],
                    "{row}: one transfer started, which took the place of the row that \
                     showed it preparing unless that was cancelled"
                );
                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}
