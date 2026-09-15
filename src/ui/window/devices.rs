// SPDX-License-Identifier: MIT

use std::{
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use gtk::{gio, prelude::*};

fn system_mounted_devices() -> Vec<PathBuf> {
    // MountEntry bindings require GLib 2.84; retain compatibility with GLib 2.72.
    std::fs::read("/proc/self/mountinfo")
        .map(|table| mounted_devices_from_table(&table))
        .unwrap_or_default()
}

fn mounted_devices_from_table(table: &[u8]) -> Vec<PathBuf> {
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
            Some(root)
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BlockCryptoHint {
    pub dm_uuid: Option<String>,
    pub id_fs_usage: Option<String>,
    pub id_fs_type: Option<String>,
    pub crypto_uuid: Option<String>,
}

impl BlockCryptoHint {
    pub(super) fn as_ref(&self) -> BlockCryptoRef<'_> {
        BlockCryptoRef {
            dm_uuid: self.dm_uuid.as_deref(),
            id_fs_usage: self.id_fs_usage.as_deref(),
            id_fs_type: self.id_fs_type.as_deref(),
            crypto_uuid: self.crypto_uuid.as_deref(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BlockCryptoRef<'a> {
    pub dm_uuid: Option<&'a str>,
    pub id_fs_usage: Option<&'a str>,
    pub id_fs_type: Option<&'a str>,
    pub crypto_uuid: Option<&'a str>,
}

// GVfs marks crypto volumes with changes-prevent/changes-allow emblems, not encrypted icon names.
pub(super) fn is_encrypted_device(
    icon_names: &[&str],
    start_stop: Option<gio::DriveStartStopType>,
    block: BlockCryptoRef<'_>,
) -> bool {
    icon_names
        .iter()
        .copied()
        .any(icon_name_indicates_encrypted)
        || start_stop == Some(gio::DriveStartStopType::Password)
        || block_indicates_crypto(block)
}

pub(super) fn encrypted_device_is_locked(icon_names: &[&str], has_mount: bool) -> bool {
    let mut padlock_locked = false;
    let mut padlock_unlocked = false;
    for name in icon_names {
        match icon_name_stem(name).as_str() {
            "changes-prevent" => padlock_locked = true,
            "changes-allow" => padlock_unlocked = true,
            _ => {}
        }
    }
    if padlock_locked {
        true
    } else if padlock_unlocked {
        false
    } else {
        !has_mount
    }
}

pub(super) fn probe_block_crypto(unix_device: &str) -> BlockCryptoHint {
    let unix_device = unix_device.trim();
    if unix_device.is_empty() {
        return BlockCryptoHint::default();
    }
    let Some(sysfs) = sysfs_block_path(unix_device) else {
        return BlockCryptoHint::default();
    };
    let udev = read_trimmed(sysfs.join("dev"))
        .and_then(|dev| std::fs::read_to_string(format!("/run/udev/data/b{dev}")).ok());
    let dm_uuid = read_trimmed(sysfs.join("dm/uuid")).or_else(|| {
        udev.as_deref()
            .and_then(|data| udev_property(data, "DM_UUID"))
    });
    let (id_fs_usage, id_fs_type) = udev
        .as_deref()
        .map(udev_crypto_properties)
        .unwrap_or((None, None));
    let crypto_uuid = dm_uuid
        .as_deref()
        .and_then(luks_uuid_from_dm_uuid)
        .or_else(|| luks_uuid_from_unix_device(unix_device))
        .or_else(|| udev.as_deref().and_then(udev_crypto_uuid))
        .or_else(|| slave_crypto_uuid(&sysfs));
    BlockCryptoHint {
        dm_uuid,
        id_fs_usage,
        id_fs_type,
        crypto_uuid,
    }
}

// An unlocked volume exposes the filesystem UUID, not the saved passphrase's LUKS UUID.
pub(super) fn crypto_password_uuid(
    volume_uuid: Option<&str>,
    unix_device: Option<&str>,
    block: BlockCryptoRef<'_>,
    locked: bool,
) -> Option<String> {
    if let Some(uuid) = block.crypto_uuid.and_then(normalize_luks_uuid) {
        return Some(uuid);
    }
    if let Some(uuid) = block.dm_uuid.and_then(luks_uuid_from_dm_uuid) {
        return Some(uuid);
    }
    if let Some(uuid) = unix_device.and_then(luks_uuid_from_unix_device) {
        return Some(uuid);
    }
    if locked {
        volume_uuid.and_then(normalize_luks_uuid)
    } else {
        None
    }
}

pub(super) fn luks_uuid_from_dm_uuid(dm_uuid: &str) -> Option<String> {
    let rest = strip_prefix_ignore_ascii_case(dm_uuid.trim(), "CRYPT-")?;
    let (kind, remainder) = rest.split_once('-')?;
    if !kind.eq_ignore_ascii_case("LUKS1") && !kind.eq_ignore_ascii_case("LUKS2") {
        return None;
    }
    take_uuid_hex(remainder)
}

pub(super) fn luks_uuid_from_unix_device(unix_device: &str) -> Option<String> {
    let name = Path::new(unix_device.trim()).file_name()?.to_str()?;
    let rest = strip_prefix_ignore_ascii_case(name, "luks-")?;
    normalize_luks_uuid(rest)
}

pub(super) fn normalize_luks_uuid(uuid: &str) -> Option<String> {
    let trimmed = uuid.trim();
    if trimmed
        .chars()
        .any(|ch| ch != '-' && !ch.is_ascii_hexdigit())
    {
        return None;
    }
    take_uuid_hex(trimmed).filter(|_| trimmed.chars().filter(|ch| *ch != '-').count() == 32)
}

fn take_uuid_hex(input: &str) -> Option<String> {
    let mut hex = String::new();
    for ch in input.chars() {
        if ch == '-' {
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            break;
        }
        hex.push(ch.to_ascii_lowercase());
        if hex.len() == 32 {
            return Some(hyphenate_uuid_hex(&hex));
        }
    }
    None
}

fn hyphenate_uuid_hex(hex32: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        &hex32[..8],
        &hex32[8..12],
        &hex32[12..16],
        &hex32[16..20],
        &hex32[20..32]
    )
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = value.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

fn icon_name_indicates_encrypted(name: &str) -> bool {
    let stem = icon_name_stem(name);
    stem.contains("encrypted") || stem == "changes-prevent" || stem == "changes-allow"
}

fn icon_name_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_suffix("-symbolic")
        .unwrap_or(&lower)
        .to_string()
}

fn block_indicates_crypto(block: BlockCryptoRef<'_>) -> bool {
    block.dm_uuid.is_some_and(dm_uuid_is_crypt)
        || block
            .id_fs_usage
            .is_some_and(|usage| usage.eq_ignore_ascii_case("crypto"))
        || block
            .id_fs_type
            .is_some_and(|kind| kind.to_ascii_lowercase().starts_with("crypto"))
}

fn dm_uuid_is_crypt(uuid: &str) -> bool {
    uuid.trim()
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("CRYPT-"))
}

fn sysfs_block_path(unix_device: &str) -> Option<PathBuf> {
    let given = Path::new(unix_device);
    let resolved = std::fs::canonicalize(given).unwrap_or_else(|_| given.to_owned());
    let name = resolved.file_name()?;
    let path = Path::new("/sys/class/block").join(name);
    path.is_dir().then_some(path)
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn udev_crypto_properties(data: &str) -> (Option<String>, Option<String>) {
    (
        udev_property(data, "ID_FS_USAGE"),
        udev_property(data, "ID_FS_TYPE"),
    )
}

fn udev_crypto_uuid(data: &str) -> Option<String> {
    let usage = udev_property(data, "ID_FS_USAGE")?;
    if !usage.eq_ignore_ascii_case("crypto") {
        return None;
    }
    udev_property(data, "ID_FS_UUID").and_then(|uuid| normalize_luks_uuid(&uuid))
}

fn slave_crypto_uuid(sysfs: &Path) -> Option<String> {
    let entries = std::fs::read_dir(sysfs.join("slaves")).ok()?;
    for entry in entries.flatten() {
        let slave = Path::new("/sys/class/block").join(entry.file_name());
        let Some(dev) = read_trimmed(slave.join("dev")) else {
            continue;
        };
        let Ok(data) = std::fs::read_to_string(format!("/run/udev/data/b{dev}")) else {
            continue;
        };
        if let Some(uuid) = udev_crypto_uuid(&data) {
            return Some(uuid);
        }
    }
    None
}

fn udev_property(data: &str, key: &str) -> Option<String> {
    let mut prefix = String::from("E:");
    prefix.push_str(key);
    prefix.push('=');
    data.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn should_list_orphaned_password_drive(
    start_stop: gio::DriveStartStopType,
    volume_covers_identity: bool,
) -> bool {
    start_stop == gio::DriveStartStopType::Password && !volume_covers_identity
}

pub(super) fn password_drive_is_orphaned(
    start_stop: gio::DriveStartStopType,
    drive_identity: Option<&str>,
    volume_identities: &[String],
    volume_claims_drive: bool,
) -> bool {
    let covered = volume_claims_drive
        || drive_identity.is_some_and(|identity| {
            volume_identities
                .iter()
                .any(|volume_identity| volume_identity == identity)
        });
    should_list_orphaned_password_drive(start_stop, covered)
}

pub(super) fn global_search_roots() -> Vec<PathBuf> {
    let monitor = gio::VolumeMonitor::get();
    let roots = monitor
        .mounts()
        .into_iter()
        .filter(|mount| !mount.is_shadowed())
        .filter_map(|mount| mount.root().path())
        .chain(system_mounted_devices());
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
