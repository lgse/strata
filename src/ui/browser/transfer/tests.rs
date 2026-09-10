// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{FileEntry, Location};
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

            view.start_transfer(
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

            view.start_transfer(
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

            view.start_transfer(
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
                wait_until(|| copied.exists(), "the non-conflicting transfer to finish");
                assert_eq!(
                    std::fs::read(&copied).expect("copied contents"),
                    std::fs::read(source_dir.join(name)).expect("source contents"),
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

            view.start_transfer(
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
            wait_until(|| kept_both.exists(), "the first Keep Both copy");
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

            view.start_transfer(
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
fn conflict_dialog_verifies_theme_following() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflict_dialog_verifies_theme_following",
        || {
            let manager = crate::ui::theme::ThemeManager::shared();
            manager.select_theme("tokyo-night");
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

            view.start_transfer(
                Location::local(&destination),
                vec![Location::local(source_dir.join("cast.txt"))],
                false,
            );
            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );

            let first = resolved_dialog_surface(&overlay);
            manager.select_theme("everforest-light-medium");
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
