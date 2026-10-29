// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::{BrowserView, PeekBehavior};

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn find_widget<T: IsA<gtk::Widget> + glib::object::IsClass>(
    root: &gtk::Widget,
    predicate: &impl Fn(&T) -> bool,
) -> Option<T> {
    if let Some(widget) = root.downcast_ref::<T>()
        && predicate(widget)
    {
        return Some(widget.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find_widget(&widget, predicate) {
            return Some(found);
        }
    }
    None
}

pub(super) fn button(root: &gtk::Widget, name: &str) -> Option<gtk::Button> {
    find_widget(root, &|button: &gtk::Button| {
        button.label().as_deref() == Some(name) && button.is_visible() && button.is_sensitive()
    })
}

pub(super) fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        if Instant::now() >= deadline {
            for window in gtk::Window::list_toplevels() {
                find_widget(&window, &|label: &gtk::Label| {
                    eprintln!("label: {}", label.text());
                    false
                });
            }
            panic!("operation timed out");
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn view() -> BrowserView {
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view
}

pub(super) fn window(view: &BrowserView) -> gtk::Window {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(1000)
        .default_height(650)
        .build();
    window.present();
    window
}

fn trashed_entry(root: &Path, name: &str, original: &Path) -> (FileEntry, PathBuf) {
    let trash = root.join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
    fs::create_dir_all(trash.join("files")).expect("files");
    fs::create_dir_all(trash.join("info")).expect("info");
    let physical = trash.join("files").join(name);
    fs::write(&physical, b"original").expect("payload");
    let info = trash.join("info").join(format!("{name}.trashinfo"));
    fs::write(
        &info,
        format!("[Trash Info]\nPath={}\n", original.display()),
    )
    .expect("metadata");
    (
        FileEntry {
            location: Location::uri(format!("trash:///{name}")),
            thumbnail_path: Some(physical),
            native_name: name.into(),
            display_name: name.into(),
            kind: crate::model::EntryKind::File,
            size: crate::model::MetadataValue::Unknown,
            modified_unix_seconds: crate::model::MetadataValue::Unknown,
            recent_unix_seconds: crate::model::MetadataValue::Unknown,
            is_hidden: false,
            mode: crate::model::MetadataValue::Unknown,
            image_dimensions: crate::model::MetadataValue::Unknown,
            child_count: crate::model::MetadataValue::Unknown,
            duration_seconds: crate::model::MetadataValue::Unknown,
            listable: crate::model::MetadataValue::Unknown,
            recent_uri: None,
        },
        info,
    )
}

#[test]
fn deletion_reach_keeps_the_local_warning_unless_every_item_is_remote() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::deletion_reach_keeps_the_local_warning_unless_every_item_is_remote",
        || {
            use DeletionReach::{Local, Remote};
            crate::adapters::mount_watch::publish_for_isolated_test(Some(
                crate::adapters::MountTable::parse(
                    "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                     2 1 0:2 / /mnt/cloud rw - fuse.rclone drive: rw\n\
                     3 1 8:33 / /media/usb rw - fuseblk /dev/sdc1 rw\n\
                     4 1 0:4 / /mnt/nas rw - nfs4 nas:/export rw\n\
                     5 1 0:5 / /mnt/pool rw - fuse.mergerfs disks rw\n",
                ),
            ));
            let entries = |locations: &[&str]| -> Vec<FileEntry> {
                locations
                    .iter()
                    .map(|location| {
                        crate::test_support::operations::entry(if location.starts_with('/') {
                            Location::local(location)
                        } else {
                            Location::uri(*location)
                        })
                    })
                    .collect()
            };
            for (locations, expected) in [
                (&["/mnt/cloud/a"][..], Remote),
                (&["/mnt/nas/a", "/mnt/cloud/b"], Remote),
                (&["sftp://host/a"], Remote),
                (&["admin:///srv/a"], Local),
                (&["/home/user/a"], Local),
                (&["/media/usb/a"], Local),
                (&["/mnt/pool/a"], Local),
                (&["TRASH:///a"], Local),
                (&["trash:///a"], Local),
                (&["/mnt/cloud/a", "/home/user/b"], Local),
                (&[], Local),
            ] {
                assert_eq!(
                    deletion_reach(&entries(locations)),
                    expected,
                    "{locations:?}"
                );
            }
        },
    );
}

#[test]
fn permanent_delete_confirmation_is_actionable_while_summary_loads() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::permanent_delete_confirmation_is_actionable_while_summary_loads",
        || {
            for (locale, confirm_text, cancel_text, warning) in [
                (
                    "en",
                    "Permanently delete 1 item",
                    "Cancel",
                    "This item will be permanently deleted. This action cannot be undone.",
                ),
                (
                    "fr",
                    "Supprimer définitivement 1 élément",
                    "Annuler",
                    "Cet élément sera supprimé définitivement. Cette action est irréversible.",
                ),
            ] {
                let fixture = tempfile::tempdir().expect("fixture");
                let original = fixture.path().join("original.txt");
                let (entry, _) = trashed_entry(fixture.path(), "Language", &original);
                let physical = entry.thumbnail_path.clone().expect("physical trash file");
                let view = view();
                let window = window(&view);
                rust_i18n::set_locale(locale);
                view.state.show_delete_confirmation(vec![entry]);

                let confirm = find_widget(window.upcast_ref(), &|button: &gtk::Button| {
                    button.label().as_deref() == Some(confirm_text)
                })
                .expect("permanent delete confirmation");
                assert!(confirm.is_sensitive());
                assert!(
                    find_widget(window.upcast_ref(), &|label: &gtk::Label| {
                        label
                            .text()
                            .split_whitespace()
                            .eq(warning.split_whitespace())
                    })
                    .is_some(),
                    "localized consequence warning: {locale}"
                );
                assert!(
                    find_widget(window.upcast_ref(), &|label: &gtk::Label| label.text()
                        == "Language")
                    .is_some(),
                    "filename must not become a translation key"
                );
                button(window.upcast_ref(), cancel_text)
                    .expect("localized cancel")
                    .emit_clicked();
                assert!(physical.exists(), "cancelling must preserve the file");
                window.close();
            }
        },
    );
}

#[test]
fn changing_metadata_after_confirmation_is_presented_refuses_the_move() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::changing_metadata_after_confirmation_is_presented_refuses_the_move",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let confirmed = fixture.path().join("confirmed");
            let changed = fixture.path().join("changed");
            let (entry, info) = trashed_entry(fixture.path(), "safe", &confirmed);
            let source = entry.thumbnail_path.clone().expect("source");
            let view = view();
            let window = window(&view);
            view.state.request_restore(vec![entry]);
            wait_until(|| button(&window.clone().upcast(), "Restore").is_some());
            fs::write(&info, format!("[Trash Info]\nPath={}\n", changed.display()))
                .expect("change metadata");
            button(&window.clone().upcast(), "Restore")
                .expect("confirm")
                .emit_clicked();
            wait_until(|| {
                find_widget(&window.clone().upcast(), &|label: &gtk::Label| {
                    label
                        .text()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .contains("no longer matches the confirmed destination")
                })
                .is_some()
            });
            assert!(source.exists());
            assert!(info.exists());
            assert!(!confirmed.exists());
            assert!(!changed.exists());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

fn normalized(label: &gtk::Label) -> String {
    label
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_label(root: &gtk::Window, text: &str) -> bool {
    find_widget(root.upcast_ref(), &|label: &gtk::Label| {
        normalized(label).contains(text)
    })
    .is_some()
}

fn focused_button_label(window: &gtk::Window) -> Option<String> {
    gtk::prelude::RootExt::focus(window)?
        .downcast::<gtk::Button>()
        .ok()?
        .label()
        .map(String::from)
}

fn delete_summary_loaded(window: &gtk::Window) -> bool {
    find_widget(window.upcast_ref(), &|label: &gtk::Label| {
        let text = normalized(label);
        text.contains(" · ") && text.ends_with(" deleted")
    })
    .is_some()
}

/// Treats `root` as an sshfs mount, as Strata sees cloud and network mounts.
fn mount_as_network_share(root: &Path) {
    crate::adapters::set_mountinfo_for_isolated_test(Some(format!(
        "1 0 8:1 / / rw - ext4 /dev/root rw\n\
         900 1 0:90 / {} rw - fuse.sshfs remote:/share rw\n",
        root.to_str().expect("UTF-8 fixture path")
    )));
}

/// Window dispatch cannot reach the modal's own key controller.
fn modal_key(window: &gtk::Window, key: gtk::gdk::Key) -> bool {
    let layer = find_widget(window.upcast_ref(), &|widget: &gtk::Widget| {
        widget.has_css_class("app-modal-layer")
    })
    .expect("open dialog");
    let controllers = layer.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("dialog key controller")
        .emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
        )
}

fn pump(milliseconds: u64) {
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn unsupported_trash_fallback_explains_and_focuses_cancel() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::unsupported_trash_fallback_explains_and_focuses_cancel",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let paths = ["a.txt", "b.txt"].map(|name| fixture.path().join(name));
            for path in &paths {
                fs::write(path, b"keep").expect("fixture file");
            }
            let entries: Vec<FileEntry> = paths
                .iter()
                .map(|path| crate::test_support::operations::entry(Location::local(path)))
                .collect();
            let held = Rc::new(crate::test_support::operations::HeldOperations::default());
            let view = view();
            view.set_operation_provider(held.clone());
            let window = window(&view);

            // `mixed`: another item failed for a different reason, so the
            // "Completed with errors" dialog's retry button comes first.
            // `remote`: a provider may keep a copy, so nothing promises permanence.
            for (remote, mixed) in [(false, false), (false, true), (true, false), (true, true)] {
                if remote {
                    mount_as_network_share(fixture.path());
                } else {
                    crate::adapters::set_mountinfo_for_isolated_test(None);
                }
                let (retry, confirm, explanation) = if remote {
                    (
                        "Delete without Trash",
                        "Delete 2 items",
                        "These items will be deleted without using Trash. Strata can't undo this; whether they can be recovered depends on the storage provider.",
                    )
                } else {
                    (
                        "Delete Permanently",
                        "Permanently delete 2 items",
                        "These items will be permanently deleted. This action cannot be undone.",
                    )
                };
                let unsupported = |id| {
                    crate::services::OperationEvent::CompletedWithErrors {
                    request_id: id,
                    deleted_locations: Vec::new(),
                    retryable_locations: entries.iter().map(|entry| entry.location.clone()).collect(),
                    has_non_retryable_failures: mixed,
                    message: "a.txt: This location doesn't support Trash.\n\n• b.txt: This location doesn't support Trash.".into(),
                }
                };
                let fail_trash_attempt = || {
                    view.state.request_delete(entries.clone(), false);
                    let id = view
                        .browser()
                        .last_started_operation()
                        .expect("trash attempt");
                    assert_eq!(
                        held.delete_requests.borrow().last().copied(),
                        Some((id, false, 2)),
                        "Delete first attempts a trash"
                    );
                    held.emit(id, unsupported(id));
                    if mixed {
                        wait_until(|| button(window.upcast_ref(), retry).is_some());
                        wait_until(|| focused_button_label(&window).as_deref() == Some(retry));
                        button(window.upcast_ref(), retry)
                            .expect("retry")
                            .emit_clicked();
                    }
                    wait_until(|| button(window.upcast_ref(), confirm).is_some());
                };

                fail_trash_attempt();
                wait_until(|| {
                    focused_button_label(&window)
                        .is_some_and(|label| label == "Cancel" || label == confirm)
                });
                let initial_focus = focused_button_label(&window);
                wait_until(|| delete_summary_loaded(&window));
                pump(300);
                assert_eq!(
                    (
                        has_label(&window, "This location doesn't support Trash."),
                        has_label(&window, explanation),
                        initial_focus.as_deref(),
                        focused_button_label(&window).as_deref(),
                    ),
                    (true, true, Some("Cancel"), Some("Cancel")),
                    "remote: {remote}, mixed: {mixed}: (explains missing Trash, describes the reach, initial focus, focus after the size summary)"
                );

                let requests = held.delete_requests.borrow().len();
                assert!(modal_key(&window, gtk::gdk::Key::Return));
                wait_until(|| button(window.upcast_ref(), confirm).is_none());
                assert_eq!(
                    held.delete_requests.borrow().len(),
                    requests,
                    "Enter cancels"
                );
                assert!(paths.iter().all(|path| path.exists()));

                fail_trash_attempt();
                button(window.upcast_ref(), confirm)
                    .expect("confirm")
                    .emit_clicked();
                wait_until(|| held.delete_requests.borrow().len() == requests + 2);
                let (id, permanent, count) = held
                    .delete_requests
                    .borrow()
                    .last()
                    .copied()
                    .expect("retry");
                assert_eq!(
                    (permanent, count),
                    (true, 2),
                    "remote: {remote}, mixed: {mixed}: confirm bypasses Trash"
                );
                held.emit(
                    id,
                    crate::services::OperationEvent::Deleted {
                        request_id: id,
                        locations: entries.iter().map(|entry| entry.location.clone()).collect(),
                    },
                );
                wait_until(|| !view.browser().is_current_operation(id));
            }
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

fn followed_file_symlink(link: &Path) -> FileEntry {
    let mut entry = glib::MainContext::new()
        .block_on(crate::adapters::query_file_entry(Location::local(link)))
        .expect("symlink entry");
    assert_eq!(entry.kind, EntryKind::FileSymbolicLink);
    entry.size = MetadataValue::Known(fs::metadata(link).expect("target metadata").len());
    entry
}

#[test]
fn trash_confirmation_sizes_the_file_link_instead_of_its_target() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::trash_confirmation_sizes_the_file_link_instead_of_its_target",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            fs::write(fixture.path().join("payload.bin"), vec![0; 100_000]).expect("target");
            let link = fixture.path().join("link");
            std::os::unix::fs::symlink("payload.bin", &link).expect("symlink");
            let view = view();
            let window = window(&view);
            view.state
                .show_delete_dialog(vec![followed_file_symlink(&link)], DeleteDialog::Trash);

            wait_until(|| has_label(&window, "11 B"));
            assert!(!has_label(&window, "100 kB"));
            button(window.upcast_ref(), "Cancel")
                .expect("cancel")
                .emit_clicked();
            assert!(link.symlink_metadata().is_ok());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn permanent_delete_counts_file_links_beyond_the_visible_rows() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::permanent_delete_counts_file_links_beyond_the_visible_rows",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            fs::write(fixture.path().join("payload.bin"), vec![0; 100_000]).expect("target");
            let count = DELETE_CONFIRMATION_MAX_ROWS + 1;
            let entries = (0..count)
                .map(|index| {
                    let link = fixture.path().join(format!("link-{index}"));
                    std::os::unix::fs::symlink("payload.bin", &link).expect("symlink");
                    followed_file_symlink(&link)
                })
                .collect();
            let view = view();
            let window = window(&view);
            view.state.show_delete_confirmation(entries);

            wait_until(|| delete_summary_loaded(&window));
            assert!(has_label(
                &window,
                &format!(
                    "{count} items · {} B will be permanently deleted",
                    count * 11
                ),
            ));
            assert!(has_label(&window, "11 B"));
            assert!(!has_label(&window, "100 kB"));
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn missing_file_link_marks_the_delete_total_incomplete() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::missing_file_link_marks_the_delete_total_incomplete",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let target = fixture.path().join("payload.bin");
            fs::write(&target, vec![0; 100_000]).expect("target");
            let link = fixture.path().join("link");
            std::os::unix::fs::symlink("payload.bin", &link).expect("symlink");
            let entry = followed_file_symlink(&link);
            fs::remove_file(&link).expect("remove link before confirmation");
            let view = view();
            let window = window(&view);
            view.state.show_delete_confirmation(vec![entry]);

            wait_until(|| delete_summary_loaded(&window));
            assert!(has_label(
                &window,
                "At least 1 item · at least 0 B will be permanently deleted"
            ));
            assert!(has_label(&window, "—"));
            assert!(!has_label(&window, "100 kB"));
            assert!(target.is_file());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn delete_as_administrator_confirms_permanent_deletion_without_trying_trash() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::delete_as_administrator_confirms_permanent_deletion_without_trying_trash",
        || {
            let entries: Vec<FileEntry> = ["a.txt", "b.txt"]
                .map(|name| {
                    crate::test_support::operations::entry(Location::uri(format!(
                        "admin:///srv/backups/{name}"
                    )))
                })
                .to_vec();
            let held = Rc::new(crate::test_support::operations::HeldOperations::default());
            let view = view();
            view.set_operation_provider(held.clone());
            let window = window(&view);

            view.state.request_delete(entries, false);

            wait_until(|| button(window.upcast_ref(), "Permanently delete 2 items").is_some());
            wait_until(|| focused_button_label(&window).as_deref() == Some("Cancel"));
            assert!(
                held.delete_requests.borrow().is_empty(),
                "Delete never attempts Trash as administrator"
            );
            assert!(has_label(
                &window,
                "Trash isn't available when browsing as administrator."
            ));
            assert!(has_label(
                &window,
                "These items will be permanently deleted. This action cannot be undone."
            ));
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn permanent_delete_symlink_to_folder_counts_only_link_and_labels_folder_link() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::permanent_delete_symlink_to_folder_counts_only_link_and_labels_folder_link",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let real_dir = fixture.path().join("realdir");
            fs::create_dir(&real_dir).expect("realdir");
            for i in 1..=5 {
                fs::write(real_dir.join(format!("file{i}")), b"x").expect("file");
            }
            let link = fixture.path().join("0dirlink");
            std::os::unix::fs::symlink(&real_dir, &link).expect("symlink");

            let context = glib::MainContext::new();
            let entry = context
                .block_on(crate::adapters::query_file_entry(Location::local(&link)))
                .expect("entry");
            assert_eq!(entry.kind, EntryKind::DirectorySymbolicLink);

            let view = view();
            let window = window(&view);
            view.state.show_delete_confirmation(vec![entry]);

            assert!(
                has_label(&window, "Folder link"),
                "dialog row must label directory symlink as Folder link"
            );

            wait_until(|| delete_summary_loaded(&window));
            let summary_label = find_widget(window.upcast_ref(), &|label: &gtk::Label| {
                let text = normalized(label);
                text.contains(" · ") && text.ends_with("will be permanently deleted")
            })
            .expect("summary label");
            assert_eq!(
                normalized(&summary_label),
                "1 item · 0 B will be permanently deleted"
            );

            let confirm =
                button(window.upcast_ref(), "Permanently delete 1 item").expect("confirm button");
            confirm.emit_clicked();
            wait_until(|| button(window.upcast_ref(), "Permanently delete 1 item").is_none());
            wait_until(|| link.symlink_metadata().is_err());
            assert!(real_dir.is_dir(), "target folder must still exist");
            for i in 1..=5 {
                assert!(
                    real_dir.join(format!("file{i}")).exists(),
                    "target file{i} must be preserved"
                );
            }

            window.destroy();
            view.browser().clear_observer();
        },
    );
}
