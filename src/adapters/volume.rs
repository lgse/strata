// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{os::unix::fs::MetadataExt, time::Duration};

use gtk::{gio, prelude::*};

use crate::{
    model::Location,
    services::{VolumeIdentity, VolumeRelation, volume_relation},
};

const REMOTE_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct DropVolumeLookup {
    pub dest: Option<VolumeIdentity>,
    pub sources: Vec<Option<VolumeIdentity>>,
    pub relation: VolumeRelation,
}

pub(crate) fn lookup_drop_volumes(
    destination: Option<&Location>,
    sources: &[Location],
    commit: bool,
) -> DropVolumeLookup {
    let cancellable =
        (commit && drop_involves_uri(destination, sources)).then(remote_volume_cancellable);
    let dest = destination
        .and_then(|destination| volume_identity(destination, true, commit, cancellable.as_ref()));
    let sources = sources
        .iter()
        .map(|source| volume_identity(source, false, commit, cancellable.as_ref()))
        .collect::<Vec<_>>();
    let relation = volume_relation(dest.as_ref(), &sources);
    DropVolumeLookup {
        dest,
        sources,
        relation,
    }
}

pub(crate) fn query_volume_identity(
    location: &Location,
    follow_symlinks: bool,
    cancellable: Option<&gio::Cancellable>,
) -> Option<VolumeIdentity> {
    if let Some(path) = location.native_path() {
        let metadata = if follow_symlinks {
            std::fs::metadata(path)
        } else {
            std::fs::symlink_metadata(path)
        };
        let metadata = metadata.ok()?;
        return Some(VolumeIdentity {
            filesystem_id: format!("dev:{}", metadata.dev()),
            is_remote: false,
        });
    }
    let owned;
    let cancellable = match cancellable {
        Some(cancellable) => cancellable,
        None => {
            owned = remote_volume_cancellable();
            &owned
        }
    };
    if cancellable.is_cancelled() {
        return None;
    }
    let file = gio_file(location);
    let info = file
        .query_info(
            gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
            query_flags(follow_symlinks),
            Some(cancellable),
        )
        .ok()?;
    if cancellable.is_cancelled() {
        return None;
    }
    let remote = file
        .query_filesystem_info(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE, Some(cancellable))
        .ok();
    identity_from_gio(location, &info, remote.as_ref())
}

pub(crate) fn location_is_remote(location: &Location) -> bool {
    match location.native_path() {
        Some(_) => false,
        None => {
            let scheme = location.backend_name();
            scheme != "file" && scheme != "trash"
        }
    }
}

fn volume_identity(
    location: &Location,
    follow_symlinks: bool,
    commit: bool,
    cancellable: Option<&gio::Cancellable>,
) -> Option<VolumeIdentity> {
    if location.native_path().is_none() && !commit {
        return None;
    }
    query_volume_identity(location, follow_symlinks, cancellable)
}

fn drop_involves_uri(destination: Option<&Location>, sources: &[Location]) -> bool {
    destination.is_some_and(|destination| destination.native_path().is_none())
        || sources.iter().any(|source| source.native_path().is_none())
}

fn remote_volume_cancellable() -> gio::Cancellable {
    cancellable_with_timeout(REMOTE_QUERY_TIMEOUT)
}

fn cancellable_with_timeout(timeout: Duration) -> gio::Cancellable {
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    // query_info is synchronous on the UI thread, so a GLib timeout cannot fire
    // until it returns. Cancel from a helper thread instead.
    let _ = std::thread::Builder::new()
        .name("strata-volume-timeout".into())
        .spawn(move || {
            std::thread::sleep(timeout);
            cancel.cancel();
        });
    cancellable
}

fn identity_from_gio(
    location: &Location,
    info: &gio::FileInfo,
    remote_info: Option<&gio::FileInfo>,
) -> Option<VolumeIdentity> {
    let filesystem_id = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM)?;
    if filesystem_id.is_empty() {
        return None;
    }
    let gio_remote = remote_info.is_some_and(|info| {
        info.has_attribute(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE)
            && info.boolean(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE)
    });
    Some(VolumeIdentity {
        filesystem_id: filesystem_id.to_string(),
        is_remote: location_is_remote(location) || gio_remote,
    })
}

fn query_flags(follow_symlinks: bool) -> gio::FileQueryInfoFlags {
    if follow_symlinks {
        gio::FileQueryInfoFlags::NONE
    } else {
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS
    }
}

fn gio_file(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}
