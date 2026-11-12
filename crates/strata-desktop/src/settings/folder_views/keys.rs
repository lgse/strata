// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::{
    adapters::{RemovableRoot, with_mount_snapshot},
    model::Location,
};

use super::{FolderKey, local_path};

/// `None` for non-native locations, network shares, and paths that cannot be stored.
/// Local FUSE overlays such as mergerfs are remembered like the drives under them.
///
/// Call it on the interface thread only: it reads `strata-fs`'s mount snapshot, which is
/// thread-local and kept current by a volume monitor's signals on that thread.
pub fn key_for_location(location: &Location) -> Option<FolderKey> {
    let path = location.native_path()?;
    with_mount_snapshot(|snapshot| {
        if snapshot.table.is_remote_share(path) {
            return None;
        }
        key_for_path(path, snapshot.removable)
    })
}

/// The innermost removable root wins, so a drive mounted inside another keeps its own key.
pub(super) fn key_for_path(path: &Path, removable: &[RemovableRoot]) -> Option<FolderKey> {
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
