// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    app::Browser,
    model::Location,
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
};

use super::{
    DeviceIds, ReleaseKey, ReleaseKind, ReleasePhase, begin_pending, clear_release_dismissed,
    complete_release_success, enter_releasing, is_pending, location_at_flush_for_test,
    note_unmount_progress, pending_device_shell, present_release_overlay, release_body_text,
    release_error_count, release_is_dismissed, release_is_pending, release_overlay_presented,
    release_phase, release_shows_spinner, reset_releases, set_mounted_roots_for_test,
    settle_pending_attempt, start_release, syncs_before_flush_for_test, unmatched_pending,
};

fn key(unix_device: &str) -> ReleaseKey {
    ReleaseKey {
        unix_device: Some(unix_device.to_owned()),
        uuid: None,
        mount_root: None,
        display_name: "USB Backup".to_owned(),
    }
}

fn host() -> (gtk::Overlay, gtk::Window) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    (overlay, window)
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        found.push(widget.clone());
        found.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    found
}

fn button_named(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    descendants(root).into_iter().find_map(|widget| {
        widget
            .downcast::<gtk::Button>()
            .ok()
            .filter(|button| button.label().as_deref() == Some(label))
    })
}

fn visible_modal(overlay: &gtk::Overlay) -> Option<gtk::Widget> {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("app-modal-layer") && !widget.has_css_class("dismissing") {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "timed out waiting for the overlay to close"
        );
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn press_escape(layer: &gtk::Widget) {
    let controllers = layer.observe_controllers();
    let escape = (0..controllers.n_items())
        .find_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .expect("escape controller");
    assert!(escape.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Escape,
            &0u32,
            &gtk::gdk::ModifierType::empty(),
        ],
    ));
}

#[test]
fn hide_keeps_the_release_in_flight() {
    crate::test_support::gtk_test(
        "ui::window::device_release::tests::hide_keeps_the_release_in_flight",
        || {
            reset_releases();
            let device = key("/dev/sdb1");
            assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
            let (overlay, window) = host();
            present_release_overlay(overlay.upcast_ref(), &device);
            let layer = visible_modal(&overlay).expect("release overlay");
            button_named(&layer, "Hide").expect("Hide").emit_clicked();
            let layer_for_close = layer.clone();
            wait_until(move || layer_for_close.parent().is_none());
            assert!(release_is_dismissed(&device));
            assert!(release_is_pending(&device));
            assert!(release_shows_spinner(&device));
            present_release_overlay(overlay.upcast_ref(), &device);
            assert!(visible_modal(&overlay).is_none());
            clear_release_dismissed(&device);
            present_release_overlay(overlay.upcast_ref(), &device);
            let layer = visible_modal(&overlay).expect("overlay after clearing dismissed");
            press_escape(&layer);
            let layer_for_escape = layer.clone();
            wait_until(move || layer_for_escape.parent().is_none());
            assert!(release_is_dismissed(&device));
            assert!(release_is_pending(&device));
            assert!(release_shows_spinner(&device));
            assert!(!release_overlay_presented(&device));
            window.destroy();
            reset_releases();
        },
    );
}

#[test]
fn rebuild_retains_a_pending_drive_volume_monitor_dropped() {
    reset_releases();
    let pending = key("/dev/sdb1");
    assert!(begin_pending(pending.clone(), ReleaseKind::Unmount));
    assert_eq!(unmatched_pending(&[]), vec![pending.clone()]);

    let live = DeviceIds {
        unix_device: Some("/dev/sdb1".to_owned()),
        uuid: None,
        mount_root: None,
    };
    assert!(is_pending(&live));
    assert!(unmatched_pending(std::slice::from_ref(&live)).is_empty());

    let other = DeviceIds {
        unix_device: Some("/dev/sdc1".to_owned()),
        uuid: None,
        mount_root: None,
    };
    assert!(!is_pending(&other));
    assert_eq!(unmatched_pending(&[other]), vec![pending]);
    reset_releases();

    let by_uuid = ReleaseKey {
        unix_device: None,
        uuid: Some("uuid-1".to_owned()),
        mount_root: None,
        display_name: "USB Backup".to_owned(),
    };
    assert!(begin_pending(by_uuid, ReleaseKind::Unmount));
    let uuid_live = DeviceIds {
        unix_device: Some("/dev/sdb1".to_owned()),
        uuid: Some("uuid-1".to_owned()),
        mount_root: None,
    };
    assert!(is_pending(&uuid_live));
    assert!(unmatched_pending(std::slice::from_ref(&uuid_live)).is_empty());
    reset_releases();

    let by_root = ReleaseKey {
        unix_device: None,
        uuid: None,
        mount_root: Some(PathBuf::from("/media/usb")),
        display_name: "USB Backup".to_owned(),
    };
    assert!(begin_pending(by_root.clone(), ReleaseKind::Unmount));
    let root_live = DeviceIds {
        unix_device: None,
        uuid: None,
        mount_root: Some(PathBuf::from("/media/usb")),
    };
    assert!(is_pending(&root_live));
    assert!(unmatched_pending(std::slice::from_ref(&root_live)).is_empty());
    assert_eq!(unmatched_pending(&[]), vec![by_root]);
    reset_releases();
}

#[test]
fn pending_row_shows_a_do_not_unplug_spinner() {
    crate::test_support::gtk_test(
        "ui::window::device_release::tests::pending_row_shows_a_do_not_unplug_spinner",
        || {
            let row = super::super::sidebar_button(crate::assets::icons::HARD_DRIVE, "USB Backup");
            let shell = pending_device_shell(&row);
            let spinner = descendants(shell.upcast_ref())
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Spinner>().ok())
                .expect("spinner");
            assert!(spinner.is_spinning());
            assert!(
                spinner
                    .tooltip_text()
                    .is_some_and(|text| text.contains("Do not unplug"))
            );
            assert!(!row.is_sensitive());
        },
    );
}

#[test]
fn flush_failure_does_not_start_unmount() {
    reset_releases();
    let device = key("/dev/sdb1");
    assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
    let calls = Cell::new(0);
    let decision =
        settle_pending_attempt(&device, Err("sync failed".to_owned()), &calls, || Ok(()));
    assert_eq!(calls.get(), 0);
    assert!(!release_is_pending(&device));
    assert_eq!(
        decision.dialog_title.as_deref(),
        Some("Unable to unmount device")
    );
    assert!(
        decision
            .dialog_message
            .as_deref()
            .is_some_and(|message| message.contains("sync failed"))
    );

    assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
    enter_releasing(&device);
    note_unmount_progress(&device, "Writing data…", 5, 1000);
    let calls = Cell::new(0);
    let decision = settle_pending_attempt(&device, Ok(()), &calls, || {
        Err(glib::Error::new(gio::IOErrorEnum::Failed, "device busy"))
    });
    assert_eq!(calls.get(), 1);
    assert!(!release_is_pending(&device));
    let message = decision.dialog_message.expect("gio error dialog");
    assert!(message.contains("device busy"));
    assert!(message.contains("Writing data…"));
    assert_eq!(
        decision.dialog_title.as_deref(),
        Some("Unable to unmount device")
    );

    assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
    let calls = Cell::new(0);
    let decision = settle_pending_attempt(&device, Ok(()), &calls, || {
        Err(glib::Error::new(gio::IOErrorEnum::Cancelled, "cancelled"))
    });
    assert_eq!(calls.get(), 1);
    assert!(decision.dialog_title.is_none());
    assert!(decision.dialog_message.is_none());
    assert!(!release_is_pending(&device));
}

#[test]
fn unmount_progress_text_is_not_a_failure() {
    reset_releases();
    let device = key("/dev/sdb1");
    assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
    enter_releasing(&device);
    note_unmount_progress(&device, "Writing data…", 5, 1000);
    let body = release_body_text(&device).expect("body");
    assert!(body.contains("Writing data…"));
    assert!(body.contains("Do not unplug"));
    assert_eq!(release_error_count(&device), 0);
    assert_eq!(release_phase(&device), Some(ReleasePhase::Releasing));
    assert!(release_is_pending(&device));

    note_unmount_progress(&device, "", 0, 0);
    let body = release_body_text(&device).expect("body");
    assert!(body.contains("Do not unplug"));
    assert!(body.contains("Writing data…"));
    assert_eq!(release_error_count(&device), 0);
    assert_eq!(release_phase(&device), Some(ReleasePhase::Releasing));
    reset_releases();
}

#[test]
fn success_clears_pending_and_keeps_a_hidden_overlay_closed() {
    crate::test_support::gtk_test(
        "ui::window::device_release::tests::success_clears_pending_and_keeps_a_hidden_overlay_closed",
        || {
            reset_releases();
            let device = key("/dev/sdb1");
            let (overlay, window) = host();
            assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
            present_release_overlay(overlay.upcast_ref(), &device);
            let hidden = visible_modal(&overlay).expect("overlay before hide");
            button_named(&hidden, "Hide").expect("Hide").emit_clicked();
            let hidden_for_close = hidden.clone();
            wait_until(move || hidden_for_close.parent().is_none());
            assert!(release_is_dismissed(&device));
            complete_release_success(&device);
            assert!(!release_is_pending(&device));
            while glib::MainContext::default().iteration(false) {}
            assert!(visible_modal(&overlay).is_none());

            assert!(begin_pending(device.clone(), ReleaseKind::Unmount));
            present_release_overlay(overlay.upcast_ref(), &device);
            let layer = visible_modal(&overlay).expect("in-flight overlay");
            complete_release_success(&device);
            assert!(!release_is_pending(&device));
            let title = descendants(&layer)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .find(|label| label.has_css_class("action-dialog-title"))
                .expect("title");
            assert_eq!(title.text().as_str(), "Safe to remove");
            let loading = descendants(&layer)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Spinner>().ok())
                .find(|spinner| spinner.has_css_class("action-dialog-loading"))
                .expect("overlay spinner");
            assert!(!loading.is_spinning());
            button_named(&layer, "Close").expect("Close").emit_clicked();
            let layer_for_close = layer.clone();
            wait_until(move || layer_for_close.parent().is_none());

            assert!(begin_pending(device.clone(), ReleaseKind::Lock));
            present_release_overlay(overlay.upcast_ref(), &device);
            let layer = visible_modal(&overlay).expect("lock overlay");
            complete_release_success(&device);
            assert!(
                descendants(&layer)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                    .all(|label| label.text().as_str() != "Safe to remove")
            );
            let layer_for_lock = layer;
            wait_until(move || layer_for_lock.parent().is_none());
            assert!(!release_is_pending(&device));
            window.destroy();
            reset_releases();
        },
    );
}

struct DropWatch {
    dropped: Arc<AtomicBool>,
}

impl FileSource for DropWatch {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(crate::services::DirectoryChange)>,
    ) -> Option<LoadHandle> {
        let dropped = Arc::clone(&self.dropped);
        Some(LoadHandle::new(move || {
            dropped.store(true, Ordering::SeqCst);
        }))
    }
}

fn label_texts(root: &gtk::Widget) -> Vec<String> {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

fn release_key(name: &str, root: PathBuf) -> ReleaseKey {
    ReleaseKey {
        unix_device: Some("/dev/sdb1".to_owned()),
        uuid: None,
        mount_root: Some(root),
        display_name: name.to_owned(),
    }
}

#[test]
fn start_release_flushes_on_a_worker_and_a_flush_error_skips_gio() {
    crate::test_support::gtk_test(
        "ui::window::device_release::tests::start_release_flushes_on_a_worker_and_a_flush_error_skips_gio",
        || {
            reset_releases();
            let main_thread = thread::current().id();
            let root = tempfile::tempdir().expect("mount root");
            let mount = root.path().to_path_buf();
            crate::adapters::install_sync_probe(Some(std::io::ErrorKind::Other), true);
            let (overlay, window) = host();
            let calls = Rc::new(Cell::new(0u32));
            let settled = Rc::new(Cell::new(false));
            let device = release_key("USB Backup", mount.clone());
            let calls_for_gio = Rc::clone(&calls);
            let settled_for_release = Rc::clone(&settled);
            assert!(start_release(
                overlay.upcast_ref(),
                None,
                None,
                device,
                ReleaseKind::Unmount,
                true,
                Some(mount.clone()),
                Some(gio::File::for_path(&mount)),
                move || settled_for_release.set(true),
                move |_| {
                    calls_for_gio.set(calls_for_gio.get().saturating_add(1));
                    async { Ok(()) }
                },
            ));
            wait_until(|| {
                crate::adapters::sync_probe_is_blocked() && visible_modal(&overlay).is_some()
            });
            let titles = label_texts(overlay.upcast_ref());
            assert!(
                titles.iter().all(|text| text != "Safe to remove"),
                "flush has not finished, so the drive is not safe to remove: {titles:?}"
            );
            let observed = crate::adapters::sync_probe_observations();
            assert!(
                observed
                    .iter()
                    .any(|call| call.thread_id != main_thread && call.root == mount)
            );
            assert_eq!(syncs_before_flush_for_test(), 0);
            crate::adapters::release_sync_probe();
            let settled_for_wait = Rc::clone(&settled);
            wait_until(move || settled_for_wait.get());
            assert_eq!(calls.get(), 0);
            let texts = label_texts(overlay.upcast_ref());
            assert!(texts.iter().all(|text| text != "Safe to remove"));
            assert!(texts.iter().any(|text| text.contains("flush failed")));
            assert!(!release_is_pending(&release_key(
                "USB Backup",
                mount.clone()
            )));
            window.destroy();
            crate::adapters::clear_sync_probe();
            reset_releases();

            let missing = mount.join("missing-mount");
            crate::adapters::install_sync_probe(None, false);
            let calls = Rc::new(Cell::new(0u32));
            let settled = Rc::new(Cell::new(false));
            let (overlay, window) = host();
            let calls_for_gio = Rc::clone(&calls);
            let settled_for_release = Rc::clone(&settled);
            assert!(start_release(
                overlay.upcast_ref(),
                None,
                None,
                release_key("USB Backup", missing.clone()),
                ReleaseKind::Unmount,
                true,
                Some(missing),
                None,
                move || settled_for_release.set(true),
                move |_| {
                    calls_for_gio.set(calls_for_gio.get().saturating_add(1));
                    async { Ok(()) }
                },
            ));
            let settled_for_wait = Rc::clone(&settled);
            wait_until(move || settled_for_wait.get());
            assert_eq!(calls.get(), 0);
            let texts = label_texts(overlay.upcast_ref());
            assert!(
                texts.iter().any(|text| text.contains("os error")),
                "open failure must be the real io error, got {texts:?}"
            );
            assert!(texts.iter().all(|text| !text.contains("Could not flush")));
            assert!(texts.iter().all(|text| text != "Safe to remove"));
            assert!(
                crate::adapters::sync_probe_observations()
                    .iter()
                    .any(|call| call.thread_id != main_thread)
            );
            window.destroy();
            crate::adapters::clear_sync_probe();
            reset_releases();

            let file = mount.join("not-a-directory");
            std::fs::write(&file, b"x").expect("file");
            crate::adapters::install_sync_probe(None, false);
            let calls = Rc::new(Cell::new(0u32));
            let settled = Rc::new(Cell::new(false));
            let (overlay, window) = host();
            let calls_for_gio = Rc::clone(&calls);
            let settled_for_release = Rc::clone(&settled);
            assert!(start_release(
                overlay.upcast_ref(),
                None,
                None,
                release_key("USB Backup", file.clone()),
                ReleaseKind::Unmount,
                true,
                Some(file),
                None,
                move || settled_for_release.set(true),
                move |_| {
                    calls_for_gio.set(calls_for_gio.get().saturating_add(1));
                    async { Ok(()) }
                },
            ));
            let settled_for_wait = Rc::clone(&settled);
            wait_until(move || settled_for_wait.get());
            assert_eq!(calls.get(), 0);
            let texts = label_texts(overlay.upcast_ref());
            assert!(
                texts.iter().any(|text| text.contains("os error")),
                "a file mount root must fail in open, got {texts:?}"
            );
            assert!(texts.iter().all(|text| !text.contains("Could not flush")));
            window.destroy();
            crate::adapters::clear_sync_probe();
            reset_releases();
        },
    );
}

#[test]
fn start_release_leaves_before_flush_and_restores_when_gio_fails_while_mounted() {
    crate::test_support::gtk_test(
        "ui::window::device_release::tests::start_release_leaves_before_flush_and_restores_when_gio_fails_while_mounted",
        || {
            reset_releases();
            let root = tempfile::tempdir().expect("mount root");
            let mount = root.path().to_path_buf();
            let dropped = Arc::new(AtomicBool::new(false));
            let browser = Browser::new(Rc::new(DropWatch {
                dropped: Arc::clone(&dropped),
            }));
            browser.navigate(Location::local(&mount));
            assert!(!dropped.load(Ordering::SeqCst));
            set_mounted_roots_for_test(Some(vec![mount.clone()]));
            crate::adapters::install_sync_probe(None, true);
            let (overlay, window) = host();
            let calls = Rc::new(Cell::new(0u32));
            let settled = Rc::new(Cell::new(false));
            let calls_for_gio = Rc::clone(&calls);
            let settled_for_release = Rc::clone(&settled);
            assert!(start_release(
                overlay.upcast_ref(),
                Some(Rc::clone(&browser)),
                None,
                release_key("USB Backup", mount.clone()),
                ReleaseKind::Unmount,
                true,
                Some(mount.clone()),
                Some(gio::File::for_path(&mount)),
                move || settled_for_release.set(true),
                move |_| {
                    calls_for_gio.set(calls_for_gio.get().saturating_add(1));
                    async { Err(glib::Error::new(gio::IOErrorEnum::Failed, "device busy")) }
                },
            ));
            wait_until(crate::adapters::sync_probe_is_blocked);
            assert!(dropped.load(Ordering::SeqCst));
            assert_eq!(syncs_before_flush_for_test(), 0);
            assert_eq!(
                location_at_flush_for_test().as_deref(),
                Some(super::super::home_directory().as_path())
            );
            assert_eq!(
                browser.active_location(),
                Some(Location::local(super::super::home_directory()))
            );
            crate::adapters::release_sync_probe();
            let settled_for_wait = Rc::clone(&settled);
            wait_until(move || settled_for_wait.get());
            assert_eq!(calls.get(), 1);
            assert_eq!(browser.active_location(), Some(Location::local(&mount)));
            window.destroy();
            crate::adapters::clear_sync_probe();
            reset_releases();

            dropped.store(false, Ordering::SeqCst);
            browser.navigate(Location::local(&mount));
            set_mounted_roots_for_test(Some(Vec::new()));
            crate::adapters::install_sync_probe(None, false);
            let (overlay, window) = host();
            let settled = Rc::new(Cell::new(false));
            let settled_for_release = Rc::clone(&settled);
            assert!(start_release(
                overlay.upcast_ref(),
                Some(Rc::clone(&browser)),
                None,
                release_key("USB Backup", mount.clone()),
                ReleaseKind::Eject,
                true,
                Some(mount),
                Some(gio::File::for_path(root.path())),
                move || settled_for_release.set(true),
                |_| async { Err(glib::Error::new(gio::IOErrorEnum::Failed, "device busy",)) },
            ));
            let settled_for_wait = Rc::clone(&settled);
            wait_until(move || settled_for_wait.get());
            assert_eq!(
                browser.active_location(),
                Some(Location::local(super::super::home_directory()))
            );
            window.destroy();
            crate::adapters::clear_sync_probe();
            set_mounted_roots_for_test(None);
            reset_releases();
        },
    );
}

#[test]
fn start_release_claims_unplug_only_when_the_drive_is_removable_or_ejectable() {
    crate::test_support::gtk_test(
        "ui::window::device_release::tests::start_release_claims_unplug_only_when_the_drive_is_removable_or_ejectable",
        || {
            reset_releases();
            let root = tempfile::tempdir().expect("mount root");
            let mount = root.path().to_path_buf();

            let run = |kind: ReleaseKind, unplug: bool| -> Vec<String> {
                let (overlay, window) = host();
                let settled = Rc::new(Cell::new(false));
                crate::adapters::install_sync_probe(None, true);
                let settled_for_release = Rc::clone(&settled);
                assert!(start_release(
                    overlay.upcast_ref(),
                    None,
                    None,
                    release_key("USB Backup", mount.clone()),
                    kind,
                    unplug,
                    Some(mount.clone()),
                    None,
                    move || settled_for_release.set(true),
                    |_| async { Ok(()) },
                ));
                wait_until(|| {
                    crate::adapters::sync_probe_is_blocked() && visible_modal(&overlay).is_some()
                });
                crate::adapters::release_sync_probe();
                let settled_for_wait = Rc::clone(&settled);
                wait_until(move || settled_for_wait.get());
                let texts = visible_modal(&overlay)
                    .map(|layer| label_texts(&layer))
                    .unwrap_or_default();
                window.destroy();
                crate::adapters::clear_sync_probe();
                reset_releases();
                texts
            };

            let texts = run(ReleaseKind::Unmount, true);
            assert!(texts.iter().any(|text| text == "Safe to remove"));
            assert!(texts.iter().any(|text| text.contains("can be unplugged")));

            let texts = run(ReleaseKind::Unmount, false);
            assert!(texts.iter().all(|text| text != "Safe to remove"));
            assert!(texts.iter().all(|text| !text.contains("can be unplugged")));

            let texts = run(ReleaseKind::Lock, true);
            assert!(texts.iter().all(|text| text != "Safe to remove"));
            assert!(texts.iter().all(|text| !text.contains("can be unplugged")));
        },
    );
}
