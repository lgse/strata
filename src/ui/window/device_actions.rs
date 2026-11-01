// SPDX-License-Identifier: MIT

//! Ejecting, unmounting and safely removing sidebar devices.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use crate::{
    adapters::{location_for_file, mount_can_unplug, volume_can_unplug},
    app::Browser,
    model::Location,
    ui::browser::BrowserView,
};

use super::{device_release, launch::home_directory};

/// Safe-removal action for a sidebar device row. Eject is preferred whenever
/// the drive reports it (USB sticks, optical discs); plain unmount covers
/// mounts without ejectable media. `None` means fixed internal storage with
/// no actionable release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MediaRelease {
    EjectVolume,
    EjectMount,
    UnmountMount,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EncryptedMediaAction {
    Unlock,
    Lock,
}

impl EncryptedMediaAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Unlock => "Unlock",
            Self::Lock => "Lock",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Unlock => crate::assets::icons::LOCK,
            Self::Lock => crate::assets::icons::LOCK_OPEN,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DeviceRowActions {
    pub(super) encrypted: Option<EncryptedMediaAction>,
    pub(super) release: Option<MediaRelease>,
}

pub(super) fn device_row_actions(
    encrypted: bool,
    mounted: bool,
    can_eject_volume: bool,
    mount_can_eject: bool,
    mount_can_unmount: bool,
) -> DeviceRowActions {
    DeviceRowActions {
        encrypted: encrypted.then_some(if mounted {
            EncryptedMediaAction::Lock
        } else {
            EncryptedMediaAction::Unlock
        }),
        release: volume_release_action(can_eject_volume, mount_can_eject, mount_can_unmount),
    }
}

pub(super) fn volume_release_action(
    can_eject_volume: bool,
    mount_can_eject: bool,
    mount_can_unmount: bool,
) -> Option<MediaRelease> {
    if can_eject_volume {
        Some(MediaRelease::EjectVolume)
    } else if mount_can_eject {
        Some(MediaRelease::EjectMount)
    } else if mount_can_unmount {
        Some(MediaRelease::UnmountMount)
    } else {
        None
    }
}

pub(super) fn media_release_label(action: MediaRelease) -> &'static str {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => "Eject",
        MediaRelease::UnmountMount => "Unmount",
    }
}

pub(super) fn media_release_error_title(action: MediaRelease) -> &'static str {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => "Unable to eject device",
        MediaRelease::UnmountMount => "Unable to unmount device",
    }
}

pub(super) fn release_kind(action: MediaRelease) -> device_release::ReleaseKind {
    match action {
        MediaRelease::UnmountMount => device_release::ReleaseKind::Unmount,
        MediaRelease::EjectVolume | MediaRelease::EjectMount => device_release::ReleaseKind::Eject,
    }
}

pub(super) fn release_unplug(
    action: &MediaRelease,
    mount: Option<&gio::Mount>,
    volume: Option<&gio::Volume>,
) -> bool {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => true,
        MediaRelease::UnmountMount => {
            mount.is_some_and(mount_can_unplug) || volume.is_some_and(volume_can_unplug)
        }
    }
}

pub(super) fn navigate_home_if_within(browser: &Rc<Browser>, root: &gio::File) {
    let Some(device) = location_for_file(root) else {
        return;
    };
    let active = browser.active_location();
    if active
        .as_ref()
        .is_some_and(|active| active == &device || active.is_within(&device))
    {
        browser.navigate(Location::local(home_directory()));
    }
}

pub(super) fn begin_media_release(in_flight: &Cell<bool>) -> bool {
    !in_flight.replace(true)
}

/// Where a device action shows progress and errors, with the guard that keeps a
/// row to one action at a time.
#[derive(Clone)]
pub(super) struct DeviceActionContext {
    pub(super) parent: gtk::Widget,
    pub(super) browser: Option<Rc<Browser>>,
    pub(super) view: BrowserView,
    pub(super) in_flight: Rc<Cell<bool>>,
}

impl DeviceActionContext {
    /// Starts `release` unless another action on the row is still running.
    pub(super) fn run_release<F, Fut>(&self, target: device_release::ReleaseTarget, run_gio: F)
    where
        F: FnOnce(gtk::MountOperation) -> Fut + 'static,
        Fut: Future<Output = Result<(), glib::Error>> + 'static,
    {
        let in_flight = self.in_flight.clone();
        let started = device_release::begin_release(
            device_release::ReleaseRequest {
                parent: &self.parent,
                browser: self.browser.clone(),
                view: Some(&self.view),
                target,
            },
            move || in_flight.set(false),
            run_gio,
        );
        if !started {
            self.in_flight.set(false);
        }
    }
}

fn release_needs_mount(action: MediaRelease) -> bool {
    matches!(
        action,
        MediaRelease::EjectMount | MediaRelease::UnmountMount
    )
}

fn not_mounted_error() -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::Failed,
        &crate::i18n::tr("The volume is not mounted."),
    )
}

async fn release_volume_media(
    action: MediaRelease,
    volume: gio::Volume,
    mount: Option<gio::Mount>,
    operation: gtk::MountOperation,
) -> Result<(), glib::Error> {
    match (action, mount) {
        (MediaRelease::EjectVolume, _) => {
            volume
                .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        }
        (_, None) => Err(not_mounted_error()),
        (_, Some(mount)) => release_mount_media(action, mount, operation).await,
    }
}

async fn release_mount_media(
    action: MediaRelease,
    mount: gio::Mount,
    operation: gtk::MountOperation,
) -> Result<(), glib::Error> {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => {
            mount
                .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        }
        MediaRelease::UnmountMount => {
            mount
                .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        }
    }
}

pub(super) fn release_device_volume(
    volume: &gio::Volume,
    action: MediaRelease,
    context: &DeviceActionContext,
) {
    if !begin_media_release(&context.in_flight) {
        return;
    }
    let mount = volume.get_mount();
    if release_needs_mount(action) && mount.is_none() {
        context.in_flight.set(false);
        return;
    }
    let target = device_release::ReleaseTarget {
        key: device_release::key_for_volume(volume),
        kind: release_kind(action),
        unplug: release_unplug(&action, mount.as_ref(), Some(volume)),
        sync_root: mount.as_ref().and_then(|mount| mount.root().path()),
        away_root: mount.as_ref().map(gio::Mount::root),
    };
    let volume = volume.clone();
    context.run_release(target, move |operation| {
        release_volume_media(action, volume, mount, operation)
    });
}

pub(super) fn release_device_mount(
    mount: &gio::Mount,
    action: MediaRelease,
    context: &DeviceActionContext,
) {
    if !begin_media_release(&context.in_flight) {
        return;
    }
    let target = device_release::ReleaseTarget {
        key: device_release::key_for_mount(mount),
        kind: release_kind(action),
        unplug: release_unplug(&action, Some(mount), None),
        sync_root: mount.root().path(),
        away_root: Some(mount.root()),
    };
    let mount = mount.clone();
    context.run_release(target, move |operation| {
        release_mount_media(action, mount, operation)
    });
}
