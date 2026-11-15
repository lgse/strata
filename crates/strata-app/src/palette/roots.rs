// SPDX-License-Identifier: MIT

//! Where the palette searches: Home, then the local drives Devices lists, as GTK's
//! `global_search_roots`.
//!
//! The rule is ported from `src/ui/window/devices.rs`, which stays where it is (A10, by the
//! coordinator's ruling; `docs/rune/parity/owner.md`, A10 item 3, asks whether it should move
//! to a shared crate). Each piece names its GTK source:
//!
//! - [`MountListing::has_own_row`]: `MountListing::has_own_row`, a mount that Devices shows
//!   on a row of its own, neither shadowed nor standing for a listed volume.
//! - [`MountRoot::local_path`]: `MountRoot::local_path`, a root with a native path, so GVfs's
//!   FUSE mirrors of remote shares are left out.
//! - [`local_device_roots`]: `local_device_roots`, the listed volumes' mounts and the mounts
//!   with a row of their own, read from the volume monitor alone ([`device_mount_roots`]), so
//!   the search skips what the backend hides, such as btrfs subvolume mounts.
//! - [`search_roots`]: `search_roots`, Home first, then the absolute roots that are neither
//!   Home, a system mount nor under GVfs's `/run/user/<uid>/gvfs`, sorted and without
//!   repeats. A drive that holds Home is kept, and so is a mount nested in another.
//!
//! The unit cases in `roots/tests.rs` are `src/ui/window/devices/tests.rs`'s for these.

use std::path::{Path, PathBuf};

use gio::prelude::*;

#[cfg(test)]
mod tests;

/// How Devices lists a mount from the volume monitor's mount list.
#[derive(Clone, Copy, Debug)]
struct MountListing {
    shadowed: bool,
    volume_listed: bool,
}

impl MountListing {
    fn of(mount: &gio::Mount, volumes: &[gio::Volume]) -> Self {
        Self {
            shadowed: mount.is_shadowed(),
            volume_listed: mount
                .volume()
                .is_some_and(|volume| volumes.contains(&volume)),
        }
    }

    /// A listed volume's row already stands for its mount.
    fn has_own_row(self) -> bool {
        !self.shadowed && !self.volume_listed
    }
}

/// A mount's root, read as `location_for_file` reads it.
#[derive(Clone, Debug)]
struct MountRoot {
    native: bool,
    path: Option<PathBuf>,
}

impl MountRoot {
    fn of(mount: &gio::Mount) -> Self {
        let root = mount.root();
        Self {
            native: root.is_native(),
            path: root.path(),
        }
    }

    /// GVfs FUSE mirrors of remote shares have a path but are not native.
    fn local_path(self) -> Option<PathBuf> {
        self.path.filter(|_| self.native)
    }
}

fn local_device_roots(
    volume_mounts: impl IntoIterator<Item = MountRoot>,
    other_mounts: impl IntoIterator<Item = (MountRoot, MountListing)>,
) -> Vec<PathBuf> {
    volume_mounts
        .into_iter()
        .chain(
            other_mounts
                .into_iter()
                .filter(|(_, listing)| listing.has_own_row())
                .map(|(root, _)| root),
        )
        .filter_map(MountRoot::local_path)
        .collect()
}

/// Taken from the volume monitor alone, so search skips what the backend hides, such as
/// btrfs subvolume mounts. On the interface thread, whose context is GLib's default.
fn device_mount_roots() -> Vec<PathBuf> {
    let monitor = gio::VolumeMonitor::get();
    let volumes = monitor.volumes();
    let volume_mounts: Vec<_> = volumes
        .iter()
        .filter_map(VolumeExt::get_mount)
        .map(|mount| MountRoot::of(&mount))
        .collect();
    let other_mounts = monitor
        .mounts()
        .into_iter()
        .map(|mount| (MountRoot::of(&mount), MountListing::of(&mount, &volumes)));
    local_device_roots(volume_mounts, other_mounts)
}

/// Home and the mounted local drives, read as the palette opens.
pub(super) fn global_search_roots() -> Vec<PathBuf> {
    search_roots(&crate::launch::home_directory(), mounted_drives())
}

/// The machine's drives, but none in this crate's tests: a hosted palette searches only the
/// test's own Home, never the drives of the machine the tests run on.
fn mounted_drives() -> Vec<PathBuf> {
    #[cfg(test)]
    return Vec::new();
    #[cfg(not(test))]
    device_mount_roots()
}

fn search_roots(home: &Path, mounts: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut mounts: Vec<_> = mounts
        .into_iter()
        .filter(|root| is_searched_mount(home, root))
        .collect();
    mounts.sort();
    mounts.dedup();
    std::iter::once(home.to_owned()).chain(mounts).collect()
}

fn is_searched_mount(home: &Path, root: &Path) -> bool {
    root.is_absolute()
        && root != home
        && !gio_unix::functions::is_mount_path_system_internal(root)
        && !is_gvfs_mirror(root)
}

/// Under `/run/user/<uid>/gvfs`, where GVfs mirrors remote locations.
fn is_gvfs_mirror(root: &Path) -> bool {
    root.strip_prefix("/run/user").is_ok_and(|relative| {
        relative
            .components()
            .nth(1)
            .is_some_and(|part| part.as_os_str() == "gvfs")
    })
}
