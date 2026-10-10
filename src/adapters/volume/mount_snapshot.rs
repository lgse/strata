// SPDX-License-Identifier: MIT

use std::{cell::RefCell, path::PathBuf};

use gtk::{gio, prelude::*};

use super::{MountTable, mount_is_removable};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemovableRoot {
    pub(crate) uuid: String,
    pub(crate) path: PathBuf,
}

/// Mount facts read once and dropped whenever mounts change, so lookups on
/// navigation never read the mount table or query GIO.
pub(crate) struct MountSnapshot {
    pub(crate) table: MountTable,
    pub(crate) removable: Vec<RemovableRoot>,
}

struct MountCache {
    snapshot: Option<MountSnapshot>,
    _monitors: (gio_unix::MountMonitor, gio::VolumeMonitor),
}

thread_local! {
    static MOUNTS: RefCell<Option<MountCache>> = const { RefCell::new(None) };
}

pub(crate) fn with_mount_snapshot<T>(read: impl FnOnce(&MountSnapshot) -> T) -> T {
    MOUNTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let cache = cache.get_or_insert_with(watch_mounts);
        read(cache.snapshot.get_or_insert_with(read_mounts))
    })
}

/// Drops the snapshot so the next read sees current mounts. GIO announces a
/// mount from an idle callback, which can run after Strata already opens a
/// drive it just mounted, so that path calls this as well.
pub(crate) fn invalidate_mount_snapshot() {
    MOUNTS.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut()
            && let Some(cache) = cache.as_mut()
        {
            cache.snapshot = None;
        }
    });
}

fn watch_mounts() -> MountCache {
    let unix = gio_unix::MountMonitor::get();
    unix.connect_mounts_changed(|_| invalidate_mount_snapshot());
    let volumes = gio::VolumeMonitor::get();
    volumes.connect_mount_added(|_, _| invalidate_mount_snapshot());
    volumes.connect_mount_removed(|_, _| invalidate_mount_snapshot());
    volumes.connect_mount_changed(|_, _| invalidate_mount_snapshot());
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
        .filter(|mount| mount_is_removable(mount))
        .filter_map(|mount| {
            Some(RemovableRoot {
                uuid: mount.volume()?.uuid()?.to_string(),
                path: mount.root().path()?,
            })
        })
        .collect();
    MountSnapshot {
        table: MountTable::current(),
        removable,
    }
}
