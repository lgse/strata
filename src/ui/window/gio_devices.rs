// SPDX-License-Identifier: MIT

//! Encryption, identity and orphan checks read from GIO volumes, mounts and drives.

use gtk::{gio, gio::prelude::EmblemedIconExt as _, glib, prelude::*};

use super::{
    device_order::{DeviceHome, DevicePlacement, DriveRank},
    devices,
};

pub(super) fn gio_icon_names(icon: &gio::Icon) -> Vec<String> {
    let mut names = Vec::new();
    collect_gio_icon_names(icon, &mut names);
    names
}

pub(super) fn collect_gio_icon_names(icon: &gio::Icon, names: &mut Vec<String>) {
    if let Ok(themed) = icon.clone().downcast::<gio::ThemedIcon>() {
        names.extend(themed.names().iter().map(|name| name.to_string()));
        return;
    }
    if let Ok(emblemed) = icon.clone().downcast::<gio::EmblemedIcon>() {
        collect_gio_icon_names(&emblemed.icon(), names);
        for emblem in emblemed.emblems() {
            collect_gio_icon_names(&emblem.icon(), names);
        }
    }
}

pub(super) fn gio_icons_are_encrypted(
    icon: &gio::Icon,
    symbolic: &gio::Icon,
    start_stop: Option<gio::DriveStartStopType>,
    unix_device: Option<&str>,
) -> bool {
    let mut names = gio_icon_names(symbolic);
    names.extend(gio_icon_names(icon));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let hint = unix_device
        .map(devices::block_crypto_hint)
        .unwrap_or_default();
    devices::is_encrypted_device(&names, start_stop, hint.as_ref())
}

pub(in crate::ui) fn gio_volume_unix_device(volume: &gio::Volume) -> Option<glib::GString> {
    volume.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
}

/// An unmounted volume the system holds cannot be mounted, so it gets no row.
pub(super) fn gio_volume_is_system_held(volume: &gio::Volume) -> bool {
    if volume.get_mount().is_some() {
        return false;
    }
    let class = volume.identifier(gio::VOLUME_IDENTIFIER_KIND_CLASS.as_str());
    devices::is_system_loop(class.as_deref(), volume.should_automount())
        || gio_volume_unix_device(volume)
            .is_some_and(|device| devices::is_exclusively_held(&device))
}

pub(crate) fn gio_volume_is_encrypted(volume: &gio::Volume) -> bool {
    gio_icons_are_encrypted(
        &volume.icon(),
        &volume.symbolic_icon(),
        volume.drive().map(|drive| drive.start_stop_type()),
        gio_volume_unix_device(volume).as_deref(),
    )
}

pub(super) fn gio_volume_is_locked(volume: &gio::Volume) -> bool {
    let mut names = gio_icon_names(&volume.symbolic_icon());
    names.extend(gio_icon_names(&volume.icon()));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let cleartext_open = gio_volume_unix_device(volume)
        .is_some_and(|device| devices::block_crypto_hint(&device).cleartext_open);
    devices::encrypted_device_is_locked(&names, volume.get_mount().is_some(), cleartext_open)
}

pub(super) fn gio_mount_is_encrypted(mount: &gio::Mount) -> bool {
    if let Some(volume) = mount.volume() {
        return gio_volume_is_encrypted(&volume);
    }
    gio_icons_are_encrypted(
        &mount.icon(),
        &mount.symbolic_icon(),
        mount.drive().map(|drive| drive.start_stop_type()),
        mount
            .drive()
            .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()))
            .as_deref(),
    )
}

pub(super) fn gio_volume_identity(volume: &gio::Volume) -> Option<String> {
    devices::device_identity(
        volume
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
            .as_deref(),
        volume.uuid().as_deref(),
        volume
            .drive()
            .and_then(|drive| gio_drive_identity(&drive))
            .as_deref(),
    )
}

pub(super) fn gio_drive_identity(drive: &gio::Drive) -> Option<String> {
    devices::device_identity(
        drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
            .as_deref(),
        drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str())
            .as_deref(),
        None,
    )
}

fn drive_home(drive: Option<gio::Drive>) -> DeviceHome {
    drive.map_or(DeviceHome::NoDrive, |drive| {
        DeviceHome::Drive(DriveRank {
            sort_key: drive
                .sort_key()
                .filter(|key| !key.is_empty())
                .map(String::from),
            name: drive.name().to_string(),
            identity: gio_drive_identity(&drive),
        })
    })
}

pub(super) fn volume_placement(volume: &gio::Volume) -> DevicePlacement {
    DevicePlacement {
        home: drive_home(volume.drive()),
        name: volume.name().to_string(),
        identity: gio_volume_identity(volume),
    }
}

pub(super) fn password_drive_placement(drive: &gio::Drive) -> DevicePlacement {
    DevicePlacement {
        home: drive_home(Some(drive.clone())),
        name: drive.name().to_string(),
        identity: gio_drive_identity(drive),
    }
}

pub(super) fn mount_placement(mount: &gio::Mount, network_share: bool) -> DevicePlacement {
    DevicePlacement {
        home: if network_share {
            DeviceHome::Network
        } else {
            drive_home(mount.drive())
        },
        name: mount.name().to_string(),
        identity: Some(mount.root().uri().to_string()),
    }
}

pub(super) fn orphaned_password_drives(
    volumes: &[gio::Volume],
    drives: &[gio::Drive],
) -> Vec<gio::Drive> {
    let identities: Vec<_> = volumes.iter().filter_map(gio_volume_identity).collect();
    drives
        .iter()
        .filter(|drive| {
            devices::password_drive_is_orphaned(
                drive.start_stop_type(),
                gio_drive_identity(drive).as_deref(),
                &identities,
                volumes
                    .iter()
                    .any(|volume| volume.drive().as_ref() == Some(*drive)),
            )
        })
        .cloned()
        .collect()
}
