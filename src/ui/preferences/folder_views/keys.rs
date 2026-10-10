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
    MOUNTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let cache = cache.get_or_insert_with(watch_mounts);
        let snapshot = cache.snapshot.get_or_insert_with(read_mounts);
        if snapshot.table.is_remote_path(path) {
            return None;
        }
        key_for_path(path, &snapshot.removable)
    })
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
    let invalidate = || {
        MOUNTS.with(|cache| {
            if let Ok(mut cache) = cache.try_borrow_mut()
                && let Some(cache) = cache.as_mut()
            {
                cache.snapshot = None;
            }
        });
    };
    let unix = gio_unix::MountMonitor::get();
    unix.connect_mounts_changed(move |_| invalidate());
    let volumes = gio::VolumeMonitor::get();
    volumes.connect_mount_added(move |_, _| invalidate());
    volumes.connect_mount_removed(move |_, _| invalidate());
    volumes.connect_mount_changed(move |_, _| invalidate());
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
