// SPDX-License-Identifier: MIT

//! Ejecting, unmounting and safely removing sidebar devices.

/// Safe-removal action for a sidebar device row. Eject is preferred whenever
/// the drive reports it (USB sticks, optical discs); plain unmount covers
/// mounts without ejectable media. `None` means fixed internal storage with
/// no actionable release.
use gtk::prelude::*;
use std::cell::Cell;
use crate::model::Location;
use std::rc::Rc;
use super::Browser;
use super::BrowserView;
use super::device_release::ReleaseKind;
use super::device_release;
use super::home_directory;
use super::location_for_file;
use super::mount_can_unplug;
use super::volume_can_unplug;
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

pub(super) fn release_device_volume(
    volume: &gio::Volume,
    action: MediaRelease,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    if !begin_media_release(in_flight) {
        return;
    }
    let mount = volume.get_mount();
    if matches!(
        action,
        MediaRelease::EjectMount | MediaRelease::UnmountMount
    ) && mount.is_none()
    {
        in_flight.set(false);
        return;
    }
    let key = device_release::key_for_volume(volume);
    let sync_root = mount.as_ref().and_then(|mount| mount.root().path());
    let away_root = mount.as_ref().map(gio::Mount::root);
    let unplug = release_unplug(&action, mount.as_ref(), Some(volume));
    let volume = volume.clone();
    let mount = mount.clone();
    let in_flight_for_settle = in_flight.clone();
    let started = device_release::start_release(
        parent,
        Some(browser.clone()),
        Some(view),
        key,
        release_kind(action),
        unplug,
        sync_root,
        away_root,
        move || in_flight_for_settle.set(false),
        move |operation| async move {
            match action {
                MediaRelease::EjectVolume => {
                    volume
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                MediaRelease::EjectMount => {
                    let Some(mount) = mount else {
                        return Err(glib::Error::new(
                            gio::IOErrorEnum::Failed,
                            &crate::i18n::tr("The volume is not mounted."),
                        ));
                    };
                    mount
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                MediaRelease::UnmountMount => {
                    let Some(mount) = mount else {
                        return Err(glib::Error::new(
                            gio::IOErrorEnum::Failed,
                            &crate::i18n::tr("The volume is not mounted."),
                        ));
                    };
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&operation),
                        )
                        .await
                }
            }
        },
    );
    if !started {
        in_flight.set(false);
    }
}

pub(super) fn release_device_mount(
    mount: &gio::Mount,
    action: MediaRelease,
    parent: &gtk::Widget,
    browser: &Rc<Browser>,
    view: &BrowserView,
    in_flight: &Rc<Cell<bool>>,
) {
    if !begin_media_release(in_flight) {
        return;
    }
    let key = device_release::key_for_mount(mount);
    let sync_root = mount.root().path();
    let away_root = mount.root();
    let unplug = release_unplug(&action, Some(mount), None);
    let mount = mount.clone();
    let in_flight_for_settle = in_flight.clone();
    let started = device_release::start_release(
        parent,
        Some(browser.clone()),
        Some(view),
        key,
        release_kind(action),
        unplug,
        sync_root,
        Some(away_root),
        move || in_flight_for_settle.set(false),
        move |operation| async move {
            match action {
                MediaRelease::EjectVolume | MediaRelease::EjectMount => {
                    mount
                        .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                        .await
                }
                MediaRelease::UnmountMount => {
                    mount
                        .unmount_with_operation_future(
                            gio::MountUnmountFlags::NONE,
                            Some(&operation),
                        )
                        .await
                }
            }
        },
    );
    if !started {
        in_flight.set(false);
    }
}
