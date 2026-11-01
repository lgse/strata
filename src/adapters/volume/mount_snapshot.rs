// SPDX-License-Identifier: MIT

use std::{cell::RefCell, path::PathBuf};

use gtk::{gio, prelude::*};

use super::{MountTable, mount_is_removable, mount_watch};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemovableRoot {
    pub(crate) uuid: String,
    pub(crate) path: PathBuf,
}

pub(crate) struct MountSnapshot<'a> {
    pub(crate) table: MountTable,
    pub(crate) removable: &'a [RemovableRoot],
}

struct MountCache {
    removable: Option<Vec<RemovableRoot>>,
    _volumes: gio::VolumeMonitor,
}

thread_local! {
    static MOUNTS: RefCell<Option<MountCache>> = const { RefCell::new(None) };
}

pub(crate) fn with_mount_snapshot<T>(read: impl FnOnce(&MountSnapshot<'_>) -> T) -> T {
    MOUNTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let cache = cache.get_or_insert_with(watch_volumes);
        read(&MountSnapshot {
            table: mount_watch::latest(),
            removable: cache.removable.get_or_insert_with(removable_roots),
        })
    })
}

/// GIO announces a mount from an idle callback, which can run after Strata
/// already opens a drive it just mounted, so that path calls this as well.
pub(crate) fn invalidate_mount_snapshot() {
    MOUNTS.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut()
            && let Some(cache) = cache.as_mut()
        {
            cache.removable = None;
        }
    });
}

fn watch_volumes() -> MountCache {
    let volumes = gio::VolumeMonitor::get();
    volumes.connect_mount_added(|_, _| invalidate_mount_snapshot());
    volumes.connect_mount_removed(|_, _| invalidate_mount_snapshot());
    volumes.connect_mount_changed(|_, _| invalidate_mount_snapshot());
    MountCache {
        removable: None,
        _volumes: volumes,
    }
}

fn removable_roots() -> Vec<RemovableRoot> {
    gio::VolumeMonitor::get()
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
        .collect()
}
