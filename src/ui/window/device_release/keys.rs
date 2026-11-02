// SPDX-License-Identifier: MIT

//! Release keys and device identities read from GIO volumes, mounts and drives.

use std::path::PathBuf;

use gtk::prelude::*;

use super::{
    super::{device_labels, drive_dialogs::PropertiesTarget},
    DeviceIds, ReleaseKey,
};
use crate::ui::preferences::PreferenceManager;

pub(in crate::ui::window) fn key_for_volume(volume: &gio::Volume) -> ReleaseKey {
    labelled(
        system_key_for_volume(volume),
        PropertiesTarget::Volume(volume.clone()),
    )
}

fn system_key_for_volume(volume: &gio::Volume) -> ReleaseKey {
    let mount = volume.get_mount();
    ReleaseKey {
        unix_device: super::super::gio_volume_unix_device(volume)
            .map(|value| value.to_string())
            .or_else(|| {
                volume
                    .drive()
                    .and_then(|drive| unix_device_identifier(&drive))
            }),
        uuid: volume.uuid().map(|value| value.to_string()),
        mount_root: mount.as_ref().and_then(mount_root_path),
        display_name: volume.name().to_string(),
    }
}

pub(in crate::ui::window) fn ids_for_volume(volume: &gio::Volume) -> DeviceIds {
    system_key_for_volume(volume).ids()
}

pub(in crate::ui::window) fn key_for_mount(mount: &gio::Mount) -> ReleaseKey {
    labelled(
        system_key_for_mount(mount),
        PropertiesTarget::Mount(mount.clone()),
    )
}

fn system_key_for_mount(mount: &gio::Mount) -> ReleaseKey {
    let volume = mount.volume();
    ReleaseKey {
        unix_device: mount
            .drive()
            .and_then(|drive| unix_device_identifier(&drive))
            .or_else(|| {
                volume
                    .as_ref()
                    .and_then(|volume| ids_for_volume(volume).unix_device)
            }),
        uuid: volume
            .as_ref()
            .and_then(|volume| volume.uuid())
            .map(|value| value.to_string())
            .or_else(|| {
                mount
                    .drive()
                    .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str()))
                    .map(|value| value.to_string())
            }),
        mount_root: mount_root_path(mount),
        display_name: mount.name().to_string(),
    }
}

pub(in crate::ui::window) fn ids_for_mount(mount: &gio::Mount) -> DeviceIds {
    system_key_for_mount(mount).ids()
}

/// Names the key as the Devices row names the device.
fn labelled(mut key: ReleaseKey, target: PropertiesTarget) -> ReleaseKey {
    key.display_name = device_labels::display_name(
        &PreferenceManager::shared(),
        target.label_id().as_deref(),
        &key.display_name,
    );
    key
}

pub(in crate::ui::window) fn key_for_drive(drive: &gio::Drive) -> ReleaseKey {
    ReleaseKey {
        unix_device: unix_device_identifier(drive),
        uuid: drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str())
            .map(|value| value.to_string()),
        mount_root: None,
        display_name: drive.name().to_string(),
    }
}

pub(in crate::ui::window) fn ids_for_drive(drive: &gio::Drive) -> DeviceIds {
    key_for_drive(drive).ids()
}

pub(super) fn unix_device_identifier(drive: &gio::Drive) -> Option<String> {
    drive
        .identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
        .map(|value| value.to_string())
}

pub(super) fn mount_root_path(mount: &gio::Mount) -> Option<PathBuf> {
    mount.root().path().map(|path| path.to_path_buf())
}
