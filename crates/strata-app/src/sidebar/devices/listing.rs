// SPDX-License-Identifier: MIT

//! What DEVICES lists, read from GIO's volume monitor as GTK's `append_devices` reads it
//! (`src/ui/window/sidebar_devices.rs`, with `gio_devices.rs` and `devices.rs`): the volumes,
//! then the mounts no listed volume stands for, each placed under its drive.
//!
//! On the interface thread, whose GLib context is the default one the hosts iterate.

use std::collections::HashSet;

use gio::prelude::*;
use strata_core::model::Location;
use strata_fs::adapters::location_for_file;

use super::order::{DeviceHome, DevicePlacement, DriveRank, group_by_drive};
use super::{MediaRelease, release_action};

/// The GIO object a row stands for.
#[derive(Clone, Debug)]
pub(super) enum Source {
    Volume(gio::Volume),
    Mount(gio::Mount),
}

/// One row of DEVICES as the monitor shows it.
#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub(super) id: String,
    /// The system's name, before any label the user gave it.
    pub(super) name: String,
    /// The key of a user's label in the settings (`label_id` in `drive_dialogs.rs`).
    pub(super) label_id: Option<String>,
    /// Where it is mounted.
    pub(super) root: Option<Location>,
    pub(super) description: String,
    pub(super) release: Option<MediaRelease>,
    pub(super) network: bool,
    pub(super) source: Source,
}

/// The rows in display order, one list per drive. A local-only sidebar (the chooser's) lists
/// only what is, or would mount as, a local path.
pub(super) fn read(local_only: bool) -> Vec<Vec<Seen>> {
    let monitor = gio::VolumeMonitor::get();
    let volumes = monitor.volumes();
    let mut rows: Vec<(DevicePlacement, Seen)> = volumes
        .iter()
        .filter(|volume| !hides_volume(volume, local_only))
        .map(|volume| (volume_placement(volume), volume_row(volume)))
        .collect();
    rows.extend(
        mounts_without_volumes(&monitor, &volumes, local_only)
            .into_iter()
            .map(|(location, mount)| {
                let network = is_network_share(&location);
                (
                    mount_placement(&mount, network),
                    mount_row(location, mount, network),
                )
            }),
    );
    group_by_drive(rows)
}

fn volume_row(volume: &gio::Volume) -> Seen {
    let mount = volume.get_mount();
    let root = mount
        .as_ref()
        .and_then(|mount| location_for_file(&mount.root()));
    let (mount_can_eject, mount_can_unmount) = mount.as_ref().map_or((false, false), |mount| {
        (mount.can_eject(), mount.can_unmount())
    });
    let uuid = volume
        .uuid()
        .or_else(|| mount.as_ref().and_then(MountExt::uuid));
    let uri = mount.as_ref().map(|mount| mount.root().uri());
    let description = root
        .as_ref()
        .map(Location::display_path)
        .or_else(|| unix_device(volume).map(String::from))
        .unwrap_or_default();
    Seen {
        id: volume_identity(volume).unwrap_or_else(|| format!("volume:{}", volume.name())),
        name: volume.name().to_string(),
        label_id: label_identity(uuid.as_deref(), uri.as_deref()),
        root,
        description,
        release: release_action(volume.can_eject(), mount_can_eject, mount_can_unmount),
        network: false,
        source: Source::Volume(volume.clone()),
    }
}

fn mount_row(location: Location, mount: gio::Mount, network: bool) -> Seen {
    let uri = mount.root().uri();
    // A share is disconnected from its menu, never ejected (`append_network_share`).
    let release = (!network)
        .then(|| release_action(false, mount.can_eject(), mount.can_unmount()))
        .flatten();
    Seen {
        id: uri.to_string(),
        name: mount.name().to_string(),
        label_id: label_identity(mount.uuid().as_deref(), Some(&uri)),
        description: location.display_path(),
        root: Some(location),
        release,
        network,
        source: Source::Mount(mount),
    }
}

/// The unshadowed mounts whose volume has no row, each once: a GVfs restart leaves the old
/// mount listed, and GIO lists the newest first.
fn mounts_without_volumes(
    monitor: &gio::VolumeMonitor,
    volumes: &[gio::Volume],
    local_only: bool,
) -> Vec<(Location, gio::Mount)> {
    let mut seen = HashSet::new();
    monitor
        .mounts()
        .into_iter()
        .filter(|mount| {
            !mount.is_shadowed()
                && !mount
                    .volume()
                    .is_some_and(|volume| volumes.contains(&volume))
        })
        .filter_map(|mount| {
            let location = location_for_file(&mount.root())?;
            let shown = !local_only || location.native_path().is_some();
            (shown && seen.insert(location.clone())).then_some((location, mount))
        })
        .collect()
}

/// GTK's `hides_volume`, less its probe for a block device another holder has claimed
/// (`is_exclusively_held`), which A3's notes leave for later.
fn hides_volume(volume: &gio::Volume, local_only: bool) -> bool {
    let mount = volume.get_mount();
    if mount.is_none() {
        let class = volume.identifier(gio::VOLUME_IDENTIFIER_KIND_CLASS.as_str());
        if is_system_loop(class.as_deref(), volume.should_automount()) {
            return true;
        }
    }
    if !local_only {
        return false;
    }
    match mount.and_then(|mount| location_for_file(&mount.root())) {
        Some(location) => location.native_path().is_none(),
        None => {
            unix_device(volume).is_none()
                || volume
                    .activation_root()
                    .is_some_and(|root| root.path().is_none())
        }
    }
}

/// GVfs marks a loop for automount only when this user attached it. Any other loop is system
/// plumbing, such as the root image of a live system (`is_system_loop` in `devices/held.rs`).
pub(super) fn is_system_loop(class: Option<&str>, should_automount: bool) -> bool {
    class == Some("loop") && !should_automount
}

fn unix_device(volume: &gio::Volume) -> Option<glib::GString> {
    volume.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
}

fn volume_identity(volume: &gio::Volume) -> Option<String> {
    device_identity(
        unix_device(volume).as_deref(),
        volume.uuid().as_deref(),
        volume
            .drive()
            .and_then(|drive| drive_identity(&drive))
            .as_deref(),
    )
}

fn drive_identity(drive: &gio::Drive) -> Option<String> {
    device_identity(
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
            identity: drive_identity(&drive),
        })
    })
}

fn volume_placement(volume: &gio::Volume) -> DevicePlacement {
    DevicePlacement {
        home: drive_home(volume.drive()),
        name: volume.name().to_string(),
        identity: volume_identity(volume),
    }
}

fn mount_placement(mount: &gio::Mount, network_share: bool) -> DevicePlacement {
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

/// The first of the device's path, its UUID and its drive's identifier that it has.
pub(super) fn device_identity(
    unix_device: Option<&str>,
    uuid: Option<&str>,
    drive_identifier: Option<&str>,
) -> Option<String> {
    [unix_device, uuid, drive_identifier]
        .into_iter()
        .find_map(|value| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

/// The settings key of a device's label: its volume's UUID, else its mount's root.
pub(super) fn label_identity(uuid: Option<&str>, root_uri: Option<&str>) -> Option<String> {
    [("volume", uuid), ("root", root_uri)]
        .into_iter()
        .find_map(|(namespace, value)| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("{namespace}:{value}"))
        })
}

/// GVfs network backends: their mounts are shares to disconnect, not drives to eject.
const NETWORK_SHARE_SCHEMES: &[&str] = &["smb", "sftp", "ftp", "ftps", "dav", "davs", "afp", "nfs"];

pub(super) fn is_network_share(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        glib::Uri::parse_scheme(uri).is_some_and(|scheme| {
            NETWORK_SHARE_SCHEMES
                .iter()
                .any(|known| scheme.eq_ignore_ascii_case(known))
        })
    })
}
