// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use gtk::{gio, prelude::*};

pub(super) struct MountedDevice {
    pub name: String,
    pub root: PathBuf,
}

pub(super) fn system_mounted_devices() -> Vec<MountedDevice> {
    // MountEntry bindings require GLib 2.84; retain compatibility with GLib 2.72.
    std::fs::read("/proc/self/mountinfo")
        .map(|table| mounted_devices_from_table(&table))
        .unwrap_or_default()
}

fn mounted_devices_from_table(table: &[u8]) -> Vec<MountedDevice> {
    table
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let fields: Vec<_> = line.split(|byte| *byte == b' ').collect();
            let separator = fields.iter().position(|field| *field == b"-")?;
            if separator < 6 {
                return None;
            }
            let root = mount_path(fields[4])?;
            let source = mount_path(fields.get(separator + 2)?)?;
            if !source.starts_with("/dev")
                || !root.is_absolute()
                || gio_unix::functions::is_mount_path_system_internal(&root)
            {
                return None;
            }
            Some(MountedDevice {
                name: root.file_name()?.to_string_lossy().into_owned(),
                root,
            })
        })
        .collect()
}

fn mount_path(encoded: &[u8]) -> Option<PathBuf> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut bytes = encoded.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            let escape = [bytes.next()?, bytes.next()?, bytes.next()?];
            decoded.push(match &escape {
                b"040" => b' ',
                b"011" => b'\t',
                b"012" => b'\n',
                b"134" => b'\\',
                _ => return None,
            });
        } else {
            decoded.push(byte);
        }
    }
    Some(std::ffi::OsString::from_vec(decoded).into())
}

pub(super) fn unrepresented_devices(
    devices: Vec<MountedDevice>,
    represented: impl IntoIterator<Item = PathBuf>,
) -> Vec<MountedDevice> {
    let mut roots: std::collections::HashSet<_> = represented.into_iter().collect();
    devices
        .into_iter()
        .filter(|device| roots.insert(device.root.clone()))
        .collect()
}

pub(super) fn global_search_roots() -> Vec<PathBuf> {
    let monitor = gio::VolumeMonitor::get();
    let roots = monitor
        .mounts()
        .into_iter()
        .filter(|mount| !mount.is_shadowed())
        .filter_map(|mount| mount.root().path())
        .chain(
            system_mounted_devices()
                .into_iter()
                .map(|device| device.root),
        );
    search_roots(&super::home_directory(), roots)
}

fn search_roots(home: &Path, mounts: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut mounts: Vec<_> = mounts
        .into_iter()
        .filter(|root| {
            root.is_absolute()
                && root != home
                && !gio_unix::functions::is_mount_path_system_internal(root)
                && !root.strip_prefix("/run/user").is_ok_and(|relative| {
                    relative
                        .components()
                        .nth(1)
                        .is_some_and(|part| part.as_os_str() == "gvfs")
                })
        })
        .collect();
    mounts.sort();
    mounts.dedup();
    std::iter::once(home.to_owned()).chain(mounts).collect()
}

#[cfg(test)]
mod tests;
