// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::{
    adapters::{RemovableRoot, with_mount_snapshot},
    model::Location,
};

use super::{FolderKey, local_path};

/// `None` for locations that are not remembered: non-native, remote, or paths
/// that cannot be stored.
pub(in crate::ui) fn key_for_location(location: &Location) -> Option<FolderKey> {
    let path = location.native_path()?;
    with_mount_snapshot(|snapshot| {
        if snapshot.table.is_network_path(path) {
            return None;
        }
        key_for_path(path, &snapshot.removable)
    })
}

pub(in crate::ui) fn holds_mount_points(directory: &Path) -> bool {
    with_mount_snapshot(|snapshot| snapshot.table.has_mount_inside(directory))
}

/// The innermost removable root wins, so a drive mounted inside another keeps its own key.
pub(in crate::ui) fn key_for_path(path: &Path, removable: &[RemovableRoot]) -> Option<FolderKey> {
    let path = local_path(path)?;
    match removable
        .iter()
        .filter(|root| path.starts_with(&root.path))
        .max_by_key(|root| root.path.components().count())
    {
        Some(root) => FolderKey::on_volume(&root.uuid, path.strip_prefix(&root.path).ok()?),
        None => Some(FolderKey::Path(path)),
    }
}
