// SPDX-License-Identifier: MIT

//! Locking encrypted devices and forgetting their cached passwords.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;

use crate::ui::{
    blur::BlurBin,
    browser::show_error_dialog,
    controls::{ModalTone, focus_button, message_dialog_description, message_dialog_layout},
    modal::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer},
};

use super::{
    device_actions::{
        DeviceActionContext, MediaRelease, begin_media_release, release_kind, release_unplug,
    },
    device_release, devices, drive_ops,
    gio_devices::{gio_volume_is_locked, gio_volume_unix_device},
    volume_password,
};

pub(super) fn lock_encrypted_volume(volume: &gio::Volume, context: &DeviceActionContext) {
    let luks_uuid = crypto_password_uuid_for_volume(volume);
    let target = lock_target(
        volume.drive(),
        gio_volume_unix_device(volume).map(Into::into),
        luks_uuid.clone(),
        device_release::key_for_volume(volume),
    );
    let request = LockRequest {
        volume_name: volume.name().to_string(),
        luks_uuid,
        mount: volume.get_mount(),
        target,
    };
    request_encrypted_lock_showing(request, context);
}

pub(super) fn lock_encrypted_mount(mount: &gio::Mount, context: &DeviceActionContext) {
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
    let request = LockRequest {
        volume_name: mount.name().to_string(),
        luks_uuid,
        mount: Some(mount.clone()),
        target,
    };
    request_encrypted_lock_showing(request, context);
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

/// The device a Lock action asks about and how it locks.
pub(super) struct LockRequest {
    pub(super) volume_name: String,
    pub(super) luks_uuid: Option<String>,
    pub(super) mount: Option<gio::Mount>,
    pub(super) target: Option<LockTarget>,
}

pub(super) fn request_encrypted_lock_showing(request: LockRequest, context: &DeviceActionContext) {
    if request.mount.is_none() && request.target.is_none() {
        show_error_dialog(
            &context.parent,
            &crate::i18n::tr("Unable to lock device"),
            &crate::i18n::tr(
                "This volume has no supported lock operation while unmounted. Mount it in Strata, then try Lock again.",
            ),
        );
        return;
    }
    if !begin_media_release(&context.in_flight) {
        return;
    }
    let context = context.clone();
    glib::MainContext::default().spawn_local(async move {
        let cached = password_is_cached(request.luks_uuid.clone()).await;
        let LockRequest {
            volume_name,
            luks_uuid,
            mount,
            target,
        } = request;
        let lock_context = context.clone();
        let start_lock = move || lock_cleartext_then_close(mount, target, &lock_context);
        let forget_context = context.clone();
        let on_lock = move || {
            glib::MainContext::default().spawn_local(async move {
                match luks_uuid.filter(|_| cached) {
                    Some(uuid) => forget_then_lock(uuid, &forget_context, start_lock).await,
                    None => start_lock(),
                }
            });
        };
        if cached {
            let cancel_in_flight = context.in_flight.clone();
            confirm_forget_cached_password(&context.parent, &volume_name, on_lock, move || {
                cancel_in_flight.set(false)
            });
        } else {
            on_lock();
        }
    });
}

async fn password_is_cached(luks_uuid: Option<String>) -> bool {
    let Some(uuid) = luks_uuid else {
        return false;
    };
    gio::spawn_blocking(move || volume_password::volume_password_is_cached(&uuid))
        .await
        .unwrap_or(false)
}

async fn forget_then_lock(uuid: String, context: &DeviceActionContext, start_lock: impl FnOnce()) {
    let forget =
        gio::spawn_blocking(move || volume_password::forget_cached_volume_password(&uuid)).await;
    let message = match forget {
        Ok(Ok(())) => return start_lock(),
        Ok(Err(error)) => error.user_message(),
        Err(_) => crate::i18n::tr("The saved password could not be deleted."),
    };
    show_error_dialog(
        &context.parent,
        &crate::i18n::tr("Couldn't forget the saved password"),
        &message,
    );
    context.in_flight.set(false);
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

    let dismiss = dismiss_once(&layer, &window_overlay, blurred_root);
    let on_confirm = Rc::new(RefCell::new(Some(on_confirm)));
    let on_cancel = Rc::new(RefCell::new(Some(on_cancel)));
    let cancelled = after_dismiss(dismiss.clone(), on_cancel.clone());
    cancel.connect_clicked(move |_| cancelled());
    let closed = after_dismiss(dismiss.clone(), on_cancel.clone());
    close.connect_clicked(move |_| closed());
    let confirmed = after_dismiss(dismiss.clone(), on_confirm);
    confirm.connect_clicked(move |_| confirmed());
    close_on_escape(&layer, after_dismiss(dismiss, on_cancel));
    focus_button(&confirm);
}

/// Dismisses the layer the first time it is called and reports whether it did.
fn dismiss_once(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    root: Option<BlurBin>,
) -> Rc<dyn Fn() -> bool> {
    let layer = layer.clone();
    let overlay = overlay.clone();
    let dismissed = Rc::new(Cell::new(false));
    Rc::new(move || {
        if dismissed.replace(true) {
            return false;
        }
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
        true
    })
}

/// Runs `handler` once, from whichever control dismissed the dialog.
fn after_dismiss(
    dismiss: Rc<dyn Fn() -> bool>,
    handler: Rc<RefCell<Option<impl FnOnce() + 'static>>>,
) -> impl Fn() + 'static {
    move || {
        if dismiss()
            && let Some(handler) = handler.borrow_mut().take()
        {
            handler();
        }
    }
}
pub(super) fn password_stop_drive(drive: Option<gio::Drive>) -> Option<gio::Drive> {
    drive.filter(|drive| {
        drive.start_stop_type() == gio::DriveStartStopType::Password && drive.can_stop()
    })
}

pub(super) fn lock_cleartext_then_close(
    mount: Option<gio::Mount>,
    target: Option<LockTarget>,
    context: &DeviceActionContext,
) {
    let key = lock_release_key(mount.as_ref(), target.as_ref());
    let release = device_release::ReleaseTarget {
        key: key.clone(),
        kind: device_release::ReleaseKind::Lock,
        unplug: false,
        sync_root: mount.as_ref().and_then(|mount| mount.root().path()),
        away_root: mount.as_ref().map(gio::Mount::root),
    };
    let parent_for_stop = context.parent.clone();
    context.run_release(release, move |unmount_operation| async move {
        if let Some(mount) = mount {
            mount
                .unmount_with_operation_future(
                    gio::MountUnmountFlags::NONE,
                    Some(&unmount_operation),
                )
                .await?;
        }
        lock_target_device(target, &parent_for_stop, &key).await
    });
}

fn lock_release_key(
    mount: Option<&gio::Mount>,
    target: Option<&LockTarget>,
) -> device_release::ReleaseKey {
    match (mount, target) {
        (Some(mount), _) => device_release::key_for_mount(mount),
        (None, Some(LockTarget::StopDrive(drive))) => device_release::key_for_drive(drive),
        (None, Some(LockTarget::Container { key, .. })) => key.clone(),
        (None, None) => device_release::ReleaseKey {
            unix_device: None,
            uuid: None,
            mount_root: None,
            display_name: String::new(),
        },
    }
}

/// Locks the encrypted container once its cleartext filesystem is unmounted.
async fn lock_target_device(
    target: Option<LockTarget>,
    parent: &gtk::Widget,
    key: &device_release::ReleaseKey,
) -> Result<(), glib::Error> {
    match target {
        Some(LockTarget::StopDrive(drive)) => {
            let stop_operation = device_release::mount_operation(parent, key);
            drive
                .stop_future(gio::MountUnmountFlags::NONE, Some(&stop_operation))
                .await
        }
        Some(LockTarget::Container {
            unix_device,
            luks_uuid,
            ..
        }) => drive_ops::lock_encrypted_device_on_system_bus(unix_device, luks_uuid).await,
        None => Ok(()),
    }
}
pub(super) fn eject_password_drive(
    drive: &gio::Drive,
    action: MediaRelease,
    context: &DeviceActionContext,
) {
    if !matches!(action, MediaRelease::EjectVolume | MediaRelease::EjectMount) {
        return;
    }
    if !begin_media_release(&context.in_flight) {
        return;
    }
    let target = device_release::ReleaseTarget {
        key: device_release::key_for_drive(drive),
        kind: release_kind(action),
        unplug: release_unplug(&action, None, None),
        sync_root: None,
        away_root: None,
    };
    let drive = drive.clone();
    context.run_release(target, move |operation| async move {
        drive
            .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
            .await
    });
}
