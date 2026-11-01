// SPDX-License-Identifier: MIT

//! Locking encrypted devices and forgetting their cached passwords.

use std::{cell::{Cell, RefCell}, rc::Rc};

use gtk::prelude::*;

use crate::{app::Browser, ui::{browser::{BrowserView, show_error_dialog}, controls::{ModalTone, focus_button, message_dialog_description, message_dialog_layout}, modal::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer}}};

use super::{device_release, devices, drive_ops, volume_password, device_actions::{MediaRelease, begin_media_release, release_kind, release_unplug}, device_release::ReleaseKind, gio_devices::{gio_volume_is_locked, gio_volume_unix_device}};

pub(super) fn lock_encrypted_volume(
    volume: &gio::Volume,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    let luks_uuid = crypto_password_uuid_for_volume(volume);
    let target = lock_target(
        volume.drive(),
        gio_volume_unix_device(volume).map(Into::into),
        luks_uuid.clone(),
        device_release::key_for_volume(volume),
    );
    request_encrypted_lock_showing(
        parent,
        &volume.name(),
        luks_uuid,
        volume.get_mount(),
        target,
        browser,
        Some(view),
        in_flight,
    );
}

pub(super) fn lock_encrypted_mount(
    mount: &gio::Mount,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    let luks_uuid = crypto_password_uuid_for_mount(mount);
    let unix_device = mount
        .volume()
        .and_then(|volume| gio_volume_unix_device(&volume))
        .map(String::from)
        .or_else(|| {
            mount
                .root()
                .path()
                .and_then(|root| drive_ops::block_device_for_path(&root))
                .map(|device| device.to_string_lossy().into_owned())
        });
    let target = lock_target(
        mount.drive(),
        unix_device,
        luks_uuid.clone(),
        device_release::key_for_mount(mount),
    );
    request_encrypted_lock_showing(
        parent,
        &mount.name(),
        luks_uuid,
        Some(mount.clone()),
        target,
        browser,
        Some(view),
        in_flight,
    );
}

pub(super) enum LockTarget {
    /// GVfs locks removable encrypted drives by stopping them.
    StopDrive(gio::Drive),
    /// Internal disks have no stoppable drive, so UDisks locks the container.
    Container {
        unix_device: Option<String>,
        luks_uuid: Option<String>,
        key: device_release::ReleaseKey,
    },
}

pub(super) fn lock_target(
    drive: Option<gio::Drive>,
    unix_device: Option<String>,
    luks_uuid: Option<String>,
    key: device_release::ReleaseKey,
) -> Option<LockTarget> {
    if let Some(drive) = password_stop_drive(drive) {
        return Some(LockTarget::StopDrive(drive));
    }
    (unix_device.is_some() || luks_uuid.is_some()).then_some(LockTarget::Container {
        unix_device,
        luks_uuid,
        key,
    })
}

pub(crate) fn crypto_password_uuid_for_volume(volume: &gio::Volume) -> Option<String> {
    let unix = gio_volume_unix_device(volume);
    let hint = unix
        .as_deref()
        .map(devices::block_crypto_hint)
        .unwrap_or_default();
    devices::crypto_password_uuid(
        volume.uuid().as_deref(),
        unix.as_deref(),
        hint.as_ref(),
        gio_volume_is_locked(volume),
    )
}

pub(super) fn crypto_password_uuid_for_mount(mount: &gio::Mount) -> Option<String> {
    if let Some(volume) = mount.volume() {
        return crypto_password_uuid_for_volume(&volume);
    }
    let unix = mount
        .drive()
        .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()));
    let hint = unix
        .as_deref()
        .map(devices::block_crypto_hint)
        .unwrap_or_default();
    devices::crypto_password_uuid(None, unix.as_deref(), hint.as_ref(), false)
}

#[expect(
    clippy::too_many_arguments,
    reason = "lock confirmation needs the volume, its mount, and the window that shows progress"
)]
pub(super) fn request_encrypted_lock_showing(
    parent: &gtk::Widget,
    volume_name: &str,
    luks_uuid: Option<String>,
    mount: Option<gio::Mount>,
    target: Option<LockTarget>,
    browser: &Rc<Browser>,
    view: Option<&BrowserView>,
    in_flight: &Rc<Cell<bool>>,
) {
    if mount.is_none() && target.is_none() {
        show_error_dialog(
            parent,
            &crate::i18n::tr("Unable to lock device"),
            &crate::i18n::tr(
                "This volume has no supported lock operation while unmounted. Mount it in Strata, then try Lock again.",
            ),
        );
        return;
    }
    if !begin_media_release(in_flight) {
        return;
    }
    let parent = parent.clone();
    let volume_name = volume_name.to_owned();
    let browser = browser.clone();
    let view = view.cloned();
    let in_flight = in_flight.clone();
    glib::MainContext::default().spawn_local(async move {
        let cached = if let Some(uuid) = luks_uuid.clone() {
            gio::spawn_blocking(move || volume_password::volume_password_is_cached(&uuid))
                .await
                .unwrap_or(false)
        } else {
            false
        };
        let parent_for_lock = parent.clone();
        let parent_for_forget = parent.clone();
        let browser_for_lock = browser.clone();
        let in_flight_for_lock = in_flight.clone();
        let in_flight_for_forget = in_flight.clone();
        let start_lock = move || {
            lock_cleartext_then_close(
                mount,
                target,
                &parent_for_lock,
                &browser_for_lock,
                view,
                &in_flight_for_lock,
            );
        };
        continue_encrypted_lock(
            &parent,
            &volume_name,
            cached,
            move || {
                glib::MainContext::default().spawn_local(async move {
                    if cached && let Some(uuid) = luks_uuid {
                        let forget = gio::spawn_blocking(move || {
                            volume_password::forget_cached_volume_password(&uuid)
                        })
                        .await;
                        match forget {
                            Ok(Ok(())) => start_lock(),
                            Ok(Err(error)) => {
                                show_error_dialog(
                                    &parent_for_forget,
                                    &crate::i18n::tr("Couldn't forget the saved password"),
                                    &error.user_message(),
                                );
                                in_flight_for_forget.set(false);
                            }
                            Err(_) => {
                                show_error_dialog(
                                    &parent_for_forget,
                                    &crate::i18n::tr("Couldn't forget the saved password"),
                                    &crate::i18n::tr("The saved password could not be deleted."),
                                );
                                in_flight_for_forget.set(false);
                            }
                        }
                    } else {
                        start_lock();
                    }
                });
            },
            move || in_flight.set(false),
        );
    });
}

pub(super) fn continue_encrypted_lock(
    parent: &gtk::Widget,
    volume_name: &str,
    password_cached: bool,
    on_lock: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    if password_cached {
        confirm_forget_cached_password(parent, volume_name, on_lock, on_cancel);
    } else {
        on_lock();
    }
}

pub(super) fn confirm_forget_cached_password(
    parent: &gtk::Widget,
    volume_name: &str,
    on_confirm: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        on_cancel();
        return;
    };

    let layout = message_dialog_layout(
        crate::assets::icons::KEY,
        &crate::i18n::tr("Forget saved password?"),
        volume_name,
        &crate::i18n::tr("Forget and lock"),
        ModalTone::Danger,
    );
    layout.body.append(&message_dialog_description(&crate::i18n::tr(
        "This volume’s password is saved. Locking will forget it, and you’ll need to enter it again to unlock.",
    )));
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let confirm = layout.confirm;

    let layer = modal_layer(
        &content,
        &window_overlay,
        blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    window_overlay.add_overlay(&layer);

    let dismissed = Rc::new(Cell::new(false));
    let on_confirm = Rc::new(RefCell::new(Some(on_confirm)));
    let on_cancel = Rc::new(RefCell::new(Some(on_cancel)));
    let dismiss = Rc::new({
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        let dismissed = dismissed.clone();
        move || {
            if dismissed.replace(true) {
                return false;
            }
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
            true
        }
    });

    let cancel_dismiss = dismiss.clone();
    let cancel_handler = on_cancel.clone();
    cancel.connect_clicked(move |_| {
        if cancel_dismiss()
            && let Some(on_cancel) = cancel_handler.borrow_mut().take()
        {
            on_cancel();
        }
    });
    let close_dismiss = dismiss.clone();
    let close_handler = on_cancel.clone();
    close.connect_clicked(move |_| {
        if close_dismiss()
            && let Some(on_cancel) = close_handler.borrow_mut().take()
        {
            on_cancel();
        }
    });
    let confirm_dismiss = dismiss.clone();
    let confirm_handler = on_confirm;
    confirm.connect_clicked(move |_| {
        if confirm_dismiss()
            && let Some(on_confirm) = confirm_handler.borrow_mut().take()
        {
            on_confirm();
        }
    });
    let escape_dismiss = dismiss;
    let escape_handler = on_cancel;
    close_on_escape(&layer, move || {
        if escape_dismiss()
            && let Some(on_cancel) = escape_handler.borrow_mut().take()
        {
            on_cancel();
        }
    });
    focus_button(&confirm);
}

pub(super) fn password_stop_drive(drive: Option<gio::Drive>) -> Option<gio::Drive> {
    drive.filter(|drive| {
        drive.start_stop_type() == gio::DriveStartStopType::Password && drive.can_stop()
    })
}

pub(super) fn lock_cleartext_then_close(
    mount: Option<gio::Mount>,
    target: Option<LockTarget>,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: Option<BrowserView>,
    in_flight: &Rc<Cell<bool>>,
) {
    let key = match (mount.as_ref(), target.as_ref()) {
        (Some(mount), _) => device_release::key_for_mount(mount),
        (None, Some(LockTarget::StopDrive(drive))) => device_release::key_for_drive(drive),
        (None, Some(LockTarget::Container { key, .. })) => key.clone(),
        (None, None) => device_release::ReleaseKey {
            unix_device: None,
            uuid: None,
            mount_root: None,
            display_name: String::new(),
        },
    };
    let sync_root = mount.as_ref().and_then(|mount| mount.root().path());
    let away_root = mount.as_ref().map(gio::Mount::root);
    let in_flight_for_settle = in_flight.clone();
    let parent_for_stop = parent.clone();
    let key_for_stop = key.clone();
    let started = device_release::start_release(
        parent,
        Some(browser.clone()),
        view.as_ref(),
        key,
        device_release::ReleaseKind::Lock,
        false,
        sync_root,
        away_root,
        move || in_flight_for_settle.set(false),
        move |unmount_operation| async move {
            let unmount_result = match mount {
                Some(mount) => {
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&unmount_operation),
                        )
                        .await
                }
                None => Ok(()),
            };
            match unmount_result {
                Ok(()) => match target {
                    Some(LockTarget::StopDrive(drive)) => {
                        let stop_operation =
                            device_release::mount_operation(&parent_for_stop, &key_for_stop);
                        drive
                            .stop_future(gio::MountUnmountFlags::NONE, Some(&stop_operation))
                            .await
                    }
                    Some(LockTarget::Container {
                        unix_device,
                        luks_uuid,
                        ..
                    }) => {
                        drive_ops::lock_encrypted_device_on_system_bus(unix_device, luks_uuid).await
                    }
                    None => Ok(()),
                },
                Err(error) => Err(error),
            }
        },
    );
    if !started {
        in_flight.set(false);
    }
}

pub(super) fn eject_password_drive(
    drive: &gio::Drive,
    action: MediaRelease,
    parent: &gtk::Widget,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    if !matches!(action, MediaRelease::EjectVolume | MediaRelease::EjectMount) {
        return;
    }
    if !begin_media_release(in_flight) {
        return;
    }
    let key = device_release::key_for_drive(drive);
    let unplug = release_unplug(&action, None, None);
    let drive = drive.clone();
    let in_flight_for_settle = in_flight.clone();
    let started = device_release::start_release(
        parent,
        None,
        Some(view),
        key,
        release_kind(action),
        unplug,
        None,
        None,
        move || in_flight_for_settle.set(false),
        move |operation| async move {
            drive
                .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        },
    );
    if !started {
        in_flight.set(false);
    }
}
