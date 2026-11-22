// SPDX-License-Identifier: MIT

//! Encrypted-volume detection, lock state, LUKS password UUID resolution, and orphaned
//! password-start drive checks (`src/ui/window/gio_devices.rs` and `src/ui/window/devices.rs`).

use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::{
    listing::{drive_identity, unix_device, volume_identity},
    probes::{self, ProbeCache},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BlockCryptoHint {
    pub(super) dm_uuid: Option<String>,
    pub(super) id_fs_usage: Option<String>,
    pub(super) id_fs_type: Option<String>,
    pub(super) crypto_uuid: Option<String>,
    /// The device is, or is held by, an open dm-crypt mapping.
    pub(super) cleartext_open: bool,
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
    pub(super) dm_uuid: Option<&'a str>,
    pub(super) id_fs_usage: Option<&'a str>,
    pub(super) id_fs_type: Option<&'a str>,
    pub(super) crypto_uuid: Option<&'a str>,
}

thread_local! {
    static CRYPTO_HINTS: ProbeCache<BlockCryptoHint> = ProbeCache::default();
}

pub(super) fn block_crypto_hint(unix_device: &str) -> BlockCryptoHint {
    probes::cached(&CRYPTO_HINTS, unix_device, probe_block_crypto)
}

pub(super) fn gio_icon_names(icon: &gio::Icon) -> Vec<String> {
    let mut names = Vec::new();
    collect_gio_icon_names(icon, &mut names);
    names
}

fn collect_gio_icon_names(icon: &gio::Icon, names: &mut Vec<String>) {
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

pub(super) fn gio_volume_is_encrypted(volume: &gio::Volume) -> bool {
    gio_icons_are_encrypted(
        &volume.icon(),
        &volume.symbolic_icon(),
        volume.drive().map(|drive| drive.start_stop_type()),
        unix_device(volume).as_deref(),
    )
}

pub(super) fn gio_volume_is_locked(volume: &gio::Volume) -> bool {
    let mut names = gio_icon_names(&volume.symbolic_icon());
    names.extend(gio_icon_names(&volume.icon()));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let cleartext_open =
        unix_device(volume).is_some_and(|device| block_crypto_hint(&device).cleartext_open);
    encrypted_device_is_locked(&names, volume.get_mount().is_some(), cleartext_open)
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

fn gio_icons_are_encrypted(
    icon: &gio::Icon,
    symbolic: &gio::Icon,
    start_stop: Option<gio::DriveStartStopType>,
    unix_device: Option<&str>,
) -> bool {
    let mut names = gio_icon_names(symbolic);
    names.extend(gio_icon_names(icon));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let hint = unix_device.map(block_crypto_hint).unwrap_or_default();
    is_encrypted_device(&names, start_stop, hint.as_ref())
}

pub(super) fn crypto_password_uuid_for_volume(volume: &gio::Volume) -> Option<String> {
    let unix = unix_device(volume);
    let hint = unix.as_deref().map(block_crypto_hint).unwrap_or_default();
    crypto_password_uuid(
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
    let hint = unix.as_deref().map(block_crypto_hint).unwrap_or_default();
    crypto_password_uuid(None, unix.as_deref(), hint.as_ref(), false)
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

/// GVfs' padlock emblem wins; without it, an open mapping whose filesystem is
/// unmounted (for example an fstab entry) still counts as unlocked.
pub(super) fn encrypted_device_is_locked(
    icon_names: &[&str],
    has_mount: bool,
    cleartext_open: bool,
) -> bool {
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
        !has_mount && !cleartext_open
    }
}

/// An unlocked volume exposes the filesystem UUID, not the saved passphrase's LUKS UUID.
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
    locked
        .then(|| volume_uuid.and_then(normalize_luks_uuid))
        .flatten()
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

pub(super) fn orphaned_password_drives(
    volumes: &[gio::Volume],
    drives: &[gio::Drive],
) -> Vec<gio::Drive> {
    let identities: Vec<_> = volumes.iter().filter_map(volume_identity).collect();
    drives
        .iter()
        .filter(|drive| {
            password_drive_is_orphaned(
                drive.start_stop_type(),
                drive_identity(drive).as_deref(),
                &identities,
                volumes
                    .iter()
                    .any(|volume| volume.drive().as_ref() == Some(*drive)),
            )
        })
        .cloned()
        .collect()
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
    start_stop == gio::DriveStartStopType::Password && !covered
}

fn probe_block_crypto(unix_device: &str) -> BlockCryptoHint {
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
        .map(|data| {
            (
                udev_property(data, "ID_FS_USAGE"),
                udev_property(data, "ID_FS_TYPE"),
            )
        })
        .unwrap_or((None, None));
    let crypto_uuid = dm_uuid
        .as_deref()
        .and_then(luks_uuid_from_dm_uuid)
        .or_else(|| luks_uuid_from_unix_device(unix_device))
        .or_else(|| udev.as_deref().and_then(udev_crypto_uuid))
        .or_else(|| slave_crypto_uuid(&sysfs));
    let cleartext_open =
        dm_uuid.as_deref().is_some_and(dm_uuid_is_crypt) || holder_is_crypt_mapping(&sysfs);
    BlockCryptoHint {
        dm_uuid,
        id_fs_usage,
        id_fs_type,
        crypto_uuid,
        cleartext_open,
    }
}

fn holder_is_crypt_mapping(sysfs: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(sysfs.join("holders")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        read_trimmed(
            Path::new("/sys/class/block")
                .join(entry.file_name())
                .join("dm/uuid"),
        )
        .is_some_and(|uuid| dm_uuid_is_crypt(&uuid))
    })
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
            return Some(format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..32]
            ));
        }
    }
    None
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
