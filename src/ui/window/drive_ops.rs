// SPDX-License-Identifier: MIT

//! Storage metadata and removable-only formatting.
//! Display labels belong to Strata preferences, not the filesystem.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use gtk::{gio, glib, prelude::*};

use crate::ui::browser::show_error_dialog;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FilesystemType {
    Fat32,
    Ntfs,
    Exfat,
}

impl FilesystemType {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Fat32 => "FAT32",
            Self::Ntfs => "NTFS",
            Self::Exfat => "exFAT",
        }
    }

    pub(super) fn max_label_len(self) -> usize {
        match self {
            Self::Fat32 => 11,
            Self::Ntfs => 32,
            Self::Exfat => 32,
        }
    }

    pub(super) fn label_character_error(self, label: &str) -> Option<String> {
        let forbidden = match self {
            Self::Fat32 => "*?.,;:/\\|+=<>[]\"",
            Self::Exfat => "*?:/\\|<>\"",
            Self::Ntfs => "",
        };
        label.chars().find_map(|character| {
            if character.is_ascii_control() {
                Some("Labels cannot contain control characters.".to_owned())
            } else if forbidden.contains(character) {
                Some(format!(
                    "{} labels cannot contain “{character}”.",
                    self.label()
                ))
            } else {
                None
            }
        })
    }

    /// Candidate executables, in preference order.
    fn mkfs_candidates(self) -> &'static [&'static str] {
        match self {
            Self::Fat32 => &["mkfs.fat"],
            Self::Ntfs => &["mkfs.ntfs", "mkntfs"],
            Self::Exfat => &["mkfs.exfat"],
        }
    }

    fn format_options(self, label: &str, quick: bool) -> glib::Variant {
        let options = glib::VariantDict::new(None);
        options.insert("update-partition-type", true);
        if !label.is_empty() {
            options.insert("label", label);
        }
        if !quick {
            options.insert("erase", "zero");
        }
        if self == Self::Fat32 {
            options.insert("mkfs-args", vec!["-F", "32"]);
        }
        options.end()
    }

    fn udisks_type(self) -> &'static str {
        match self {
            Self::Fat32 => "vfat",
            Self::Ntfs => "ntfs",
            Self::Exfat => "exfat",
        }
    }

    fn resolve_mkfs(self) -> Option<PathBuf> {
        self.mkfs_candidates().iter().find_map(|cmd| tool_path(cmd))
    }

    pub(super) fn format_tool_name(self) -> &'static str {
        match self {
            Self::Fat32 => "mkfs.fat",
            Self::Ntfs => "mkfs.ntfs or mkntfs",
            Self::Exfat => "mkfs.exfat",
        }
    }

    pub(super) fn available(self) -> bool {
        self.resolve_mkfs().is_some()
    }
}

pub(super) fn show_mount(volume: Option<&gio::Volume>) -> bool {
    is_eligible(volume)
        && volume.is_some_and(|volume| volume.get_mount().is_none() && volume.can_mount())
}

/// Eligible targets are removable drives only. Network shares, internal
/// fixed disks, and volumes without a drive association are excluded.
pub(super) fn is_eligible(volume: Option<&gio::Volume>) -> bool {
    if let Some(volume) = volume
        && let Some(drive) = volume.drive()
    {
        return drive.is_removable() || drive.is_media_removable();
    }
    false
}

#[derive(Debug)]
pub(super) enum DriveOpError {
    CommandFailed(String),
    DeviceNotFound,
    Cancelled,
    InvalidLabel(String),
    Io(std::io::Error),
}

impl std::fmt::Display for DriveOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandFailed(msg) => write!(f, "{msg}"),
            Self::DeviceNotFound => f.write_str("Could not identify the drive's block device"),
            Self::Cancelled => f.write_str("Operation cancelled"),
            Self::InvalidLabel(msg) => write!(f, "Invalid label: {msg}"),
            Self::Io(error) => write!(f, "I/O error: {error}"),
        }
    }
}

impl std::error::Error for DriveOpError {}

impl From<std::io::Error> for DriveOpError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<glib::Error> for DriveOpError {
    fn from(error: glib::Error) -> Self {
        if gio::DBusError::remote_error(&error)
            .is_some_and(|name| name == "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed")
            || error.matches(gio::IOErrorEnum::Cancelled)
            || error.matches(gio::IOErrorEnum::FailedHandled)
        {
            Self::Cancelled
        } else {
            Self::CommandFailed(error.message().to_string())
        }
    }
}

/// Locate an executable through `PATH` plus the usual sbin directories.
fn tool_path(cmd: &str) -> Option<PathBuf> {
    if cmd.contains('/') {
        let path = PathBuf::from(cmd);
        return is_executable(&path).then_some(path);
    }
    if let Some(dirs) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&dirs) {
            let candidate = dir.join(cmd);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    for dir in ["/sbin", "/usr/sbin", "/usr/local/sbin"] {
        let candidate = Path::new(dir).join(cmd);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

pub(super) fn block_device_for_volume(volume: &gio::Volume) -> Option<PathBuf> {
    if let Some(device) = super::gio_volume_unix_device(volume) {
        return Some(PathBuf::from(device.as_str()));
    }
    None
}

/// Total and available bytes for a mounted path, or `None` when the
/// filesystem does not report capacity.
pub(super) fn usage_for_path(path: &Path) -> Option<(u64, u64)> {
    let stat = rustix::fs::statvfs(path).ok()?;
    if stat.f_blocks == 0 {
        return None;
    }
    let block = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize.max(1)
    };
    Some((
        stat.f_blocks.saturating_mul(block),
        stat.f_bavail.saturating_mul(block),
    ))
}

pub(super) fn block_device_for_path(path: &Path) -> Option<PathBuf> {
    let table = std::fs::read("/proc/self/mountinfo").ok()?;
    super::devices::block_device_from_mount_table(&table, path)
}

pub(super) fn mounted_path_for_device(device: &Path) -> Option<PathBuf> {
    let table = std::fs::read("/proc/self/mountinfo").ok()?;
    super::devices::mounted_path_from_table(&table, device)
}

/// Human-readable filesystem name for a block device.
pub(super) fn filesystem_label_for_device(device: &Path) -> Option<String> {
    let fstype = tool_path("lsblk").and_then(|exe| {
        Command::new(exe)
            .args(["-n", "-o", "FSTYPE", &device.to_string_lossy()])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    })?;
    match fstype.as_str() {
        "vfat" | "fat" | "msdos" => Some("FAT32".to_owned()),
        "ntfs" => Some("NTFS".to_owned()),
        "exfat" => Some("exFAT".to_owned()),
        "ext4" => Some("ext4".to_owned()),
        "btrfs" => Some("btrfs".to_owned()),
        "" => None,
        other => Some(other.to_owned()),
    }
}

/// Total size in bytes for a block device, via `lsblk`. Works whether or
/// not the volume is mounted.
pub(super) fn device_size_bytes(device: &Path) -> Option<u64> {
    let exe = tool_path("lsblk")?;
    let output = Command::new(exe)
        .args(["-b", "-n", "-o", "SIZE", &device.to_string_lossy()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
}

/// Unmount the volume when mounted: the label/format tools need exclusive
/// access to the block device.
async fn unmount_for_exclusive_access(
    parent: &gtk::Widget,
    volume: &gio::Volume,
) -> Result<PathBuf, DriveOpError> {
    let device = block_device_for_volume(volume).ok_or(DriveOpError::DeviceNotFound)?;
    let Some(mount) = volume.get_mount() else {
        return Ok(device);
    };
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    mount
        .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
        .await
        .map_err(DriveOpError::from)?;
    Ok(device)
}

/// Format the volume's block device.
pub(super) async fn format_volume(
    parent: gtk::Widget,
    volume: gio::Volume,
    fs_type: FilesystemType,
    label: String,
    quick: bool,
) -> Result<(), DriveOpError> {
    if !is_eligible(Some(&volume)) {
        return Err(DriveOpError::DeviceNotFound);
    }
    if label.chars().count() > fs_type.max_label_len() {
        return Err(DriveOpError::InvalidLabel(
            "Filesystem label is too long.".to_owned(),
        ));
    }
    if let Some(message) = fs_type.label_character_error(&label) {
        return Err(DriveOpError::InvalidLabel(message));
    }
    let device = unmount_for_exclusive_access(&parent, &volume).await?;
    gio::spawn_blocking(move || {
        let connection = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)?;
        format_device(
            &device,
            fs_type,
            &label,
            quick,
            |path, interface, method, parameters| {
                connection
                    .call_sync(
                        Some("org.freedesktop.UDisks2"),
                        path,
                        interface,
                        method,
                        Some(parameters),
                        None,
                        gio::DBusCallFlags::NONE,
                        i32::MAX,
                        gio::Cancellable::NONE,
                    )
                    .map_err(DriveOpError::from)
            },
        )
    })
    .await
    .map_err(|_| DriveOpError::CommandFailed("Formatting task did not complete".to_owned()))?
}

fn format_device(
    device: &Path,
    fs_type: FilesystemType,
    label: &str,
    quick: bool,
    mut call: impl FnMut(&str, &str, &str, &glib::Variant) -> Result<glib::Variant, DriveOpError>,
) -> Result<(), DriveOpError> {
    let device = device.to_str().ok_or(DriveOpError::DeviceNotFound)?;
    let spec = glib::VariantDict::new(None);
    spec.insert("path", device);
    let options = glib::VariantDict::new(None).end();
    let resolved = call(
        "/org/freedesktop/UDisks2/Manager",
        "org.freedesktop.UDisks2.Manager",
        "ResolveDevice",
        &glib::Variant::tuple_from_iter([spec.end(), options]),
    )?;
    let (paths,) = resolved
        .get::<(Vec<glib::variant::ObjectPath>,)>()
        .ok_or(DriveOpError::DeviceNotFound)?;
    let [path] = paths.as_slice() else {
        return Err(DriveOpError::DeviceNotFound);
    };
    // UDisks updates the partition type and reprobes the new filesystem together.
    call(
        path.as_str(),
        "org.freedesktop.UDisks2.Block",
        "Format",
        &glib::Variant::tuple_from_iter([
            fs_type.udisks_type().to_variant(),
            fs_type.format_options(label, quick),
        ]),
    )?;
    Ok(())
}

/// Report the outcome: stay quiet on cancellation and success, and show
/// everything else through the error dialog.
pub(super) fn report_result(
    parent: &gtk::Widget,
    display_name: &str,
    result: Result<(), DriveOpError>,
) {
    match result {
        Ok(()) => {}
        Err(DriveOpError::Cancelled) => {}
        Err(error) => show_error_dialog(
            parent,
            &format!("Unable to update {display_name}"),
            &error.to_string(),
        ),
    }
}

#[cfg(test)]
mod tests;
