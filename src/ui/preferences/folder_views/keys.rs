// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use gtk::{gio, prelude::*};

use crate::{adapters::MountTable, model::Location};

use super::FolderKey;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::ui) struct RemovableRoot {
    pub(in crate::ui) uuid: String,
    pub(in crate::ui) path: PathBuf,
}

/// Mount facts read once and dropped whenever mounts change, so resolving a
/// key on navigation never reads the mount table or queries GIO.
struct MountSnapshot {
    table: MountTable,
    removable: Vec<RemovableRoot>,
}

struct MountCache {
    snapshot: Option<MountSnapshot>,
    _monitors: (gio_unix::MountMonitor, gio::VolumeMonitor),
}

thread_local! {
    static MOUNTS: RefCell<Option<MountCache>> = const { RefCell::new(None) };
}

/// `None` for locations that are not remembered: non-native, remote, or paths
/// that cannot be stored.
pub(in crate::ui) fn key_for_location(location: &Location) -> Option<FolderKey> {
    let path = location.native_path()?;
    with_mounts(|snapshot| {
        if snapshot.table.is_network_path(path) {
            return None;
        }
        key_for_path(path, &snapshot.removable)
    })
}

/// Whether a filesystem is mounted directly inside `directory`.
pub(in crate::ui) fn holds_mount_points(directory: &Path) -> bool {
    with_mounts(|snapshot| snapshot.table.has_mount_inside(directory))
}

fn with_mounts<T>(read: impl FnOnce(&MountSnapshot) -> T) -> T {
    MOUNTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let cache = cache.get_or_insert_with(watch_mounts);
        read(cache.snapshot.get_or_insert_with(read_mounts))
    })
}

/// Drops the snapshot so the next key reads mounts again. GIO announces a
/// mount from an idle callback, which can run after Strata already opens a
/// drive it just mounted, so that path calls this as well.
pub(in crate::ui) fn reread_mounts() {
    MOUNTS.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut()
            && let Some(cache) = cache.as_mut()
        {
            cache.snapshot = None;
        }
    });
}

/// The innermost removable root wins, so a drive mounted inside another keeps its own key.
pub(in crate::ui) fn key_for_path(path: &Path, removable: &[RemovableRoot]) -> Option<FolderKey> {
    let FolderKey::Path(path) = FolderKey::local(path)? else {
        return None;
    };
    match removable
        .iter()
        .filter(|root| path.starts_with(&root.path))
        .max_by_key(|root| root.path.components().count())
    {
        Some(root) => FolderKey::on_volume(&root.uuid, path.strip_prefix(&root.path).ok()?),
        None => Some(FolderKey::Path(path)),
    }
}

fn watch_mounts() -> MountCache {
    let unix = gio_unix::MountMonitor::get();
    unix.connect_mounts_changed(|_| reread_mounts());
    let volumes = gio::VolumeMonitor::get();
    volumes.connect_mount_added(|_, _| reread_mounts());
    volumes.connect_mount_removed(|_, _| reread_mounts());
    volumes.connect_mount_changed(|_, _| reread_mounts());
    MountCache {
        snapshot: None,
        _monitors: (unix, volumes),
    }
}

fn read_mounts() -> MountSnapshot {
    let removable = gio::VolumeMonitor::get()
        .mounts()
        .iter()
        .filter(|mount| !mount.is_shadowed())
        .filter_map(|mount| {
            let volume = mount.volume()?;
            if !crate::ui::window::mount_can_unplug(mount)
                && !crate::ui::window::volume_can_unplug(&volume)
            {
                return None;
            }
            Some(RemovableRoot {
                uuid: volume.uuid()?.to_string(),
                path: mount.root().path()?,
            })
        })
        .collect();
    MountSnapshot {
        table: MountTable::current(),
        removable,
    }
}
