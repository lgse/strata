// SPDX-License-Identifier: MIT

mod mount_capabilities;
pub(crate) mod mount_lanes;
mod mount_snapshot;
pub(crate) mod mount_watch;
mod mounts;
mod names;
#[cfg(test)]
mod tests;

use std::{cell::RefCell, collections::HashSet, path::Path, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::gio_file_for_location;
use crate::{
    model::Location,
    services::{VolumeIdentity, VolumeRelation, volume_relation},
};

#[cfg(test)]
pub(crate) use mount_capabilities::forget_test_mount_capabilities;
pub(crate) use mount_capabilities::{
    MountCapabilities, MountKey, XattrNamespace, learn_mount_capability, mount_capabilities,
    teach_mount_capability,
};
pub(crate) use mount_snapshot::{RemovableRoot, invalidate_mount_snapshot, with_mount_snapshot};
#[cfg(test)]
pub(crate) use mounts::FAT32_MAX_FILE_SIZE;
#[cfg(test)]
pub(crate) use mounts::set_mountinfo_for_isolated_test;
pub(crate) use mounts::{
    CoarseTimes, Family, FuseFormat, LONGEST_NAME_NTFS_3G_RENAMES_OVER, MountKey as TrashMountKey,
    MountSite, MountTable, MountType, SillyNames, family, holds_symlinks, is_fuse_fs_type,
    is_local_fuse_overlay, max_file_size,
};
pub(crate) use names::{
    MountNames, NameEncoding, NameLimit, NameProfile, NameRules, PrivateUse, Reserved, StrictRule,
    Strictness, Strip, gvfs_bridge_profile, gvfs_profile,
};

pub(crate) const REMOTE_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DropVolumeQuery {
    pub dest: Location,
    pub source_parents: Vec<Location>,
}

impl DropVolumeQuery {
    pub(crate) fn new(dest: &Location, sources: &[Location]) -> Self {
        Self::with_mounts(dest, sources, &mount_watch::latest())
    }

    fn with_mounts(dest: &Location, sources: &[Location], mounts: &MountTable) -> Self {
        let mut seen = HashSet::new();
        let mut source_parents = Vec::new();
        let mut last_plain_parent: Option<&Path> = None;
        for source in sources {
            let parent_path = source.native_path().and_then(Path::parent);
            // A directory without mount points directly inside is every child's volume directory.
            if parent_path.is_some() && parent_path == last_plain_parent {
                continue;
            }
            let parent = source_volume_directory(source, mounts);
            if let Some(parent_path) = parent_path
                && parent.native_path() == Some(parent_path)
                && !mounts.has_mount_inside(parent_path)
            {
                last_plain_parent = Some(parent_path);
            }
            if seen.insert(parent.clone()) {
                source_parents.push(parent);
            }
        }
        Self {
            dest: dest.clone(),
            source_parents,
        }
    }

    fn locations(&self) -> impl Iterator<Item = &Location> {
        std::iter::once(&self.dest).chain(self.source_parents.iter())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DropVolumeLookup {
    pub dest: Option<VolumeIdentity>,
    pub sources: Vec<Option<VolumeIdentity>>,
    pub relation: VolumeRelation,
}

impl DropVolumeLookup {
    fn from_identities(identities: Vec<Option<VolumeIdentity>>) -> Self {
        let mut identities = identities.into_iter();
        let dest = identities.next().flatten();
        let sources = identities.collect::<Vec<_>>();
        let relation = volume_relation(dest.as_ref(), &sources);
        Self {
            dest,
            sources,
            relation,
        }
    }

    pub(crate) fn describe(&self) -> String {
        let id = |identity: &Option<VolumeIdentity>| {
            identity
                .as_ref()
                .map_or("?", |identity| identity.filesystem_id.as_str())
                .to_owned()
        };
        let sources = self.sources.iter().map(id).collect::<Vec<_>>();
        format!("dest={} sources=[{}]", id(&self.dest), sources.join(", "))
    }
}

pub(crate) enum DropVolumes {
    Ready(DropVolumeLookup),
    Pending(PendingVolumeLookup),
}

impl DropVolumes {
    pub(crate) fn relation(&self) -> VolumeRelation {
        match self {
            Self::Ready(lookup) => lookup.relation,
            Self::Pending(_) => VolumeRelation::Unknown,
        }
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Ready(lookup) => lookup.describe(),
            Self::Pending(pending) if pending.cancellable.is_cancelled() => "timed out".into(),
            Self::Pending(_) => "pending".into(),
        }
    }
}

/// Calls `on_ready` once on the main context unless the pending handle is dropped.
pub(crate) fn lookup_drop_volumes(
    query: &DropVolumeQuery,
    on_ready: impl FnOnce(DropVolumeLookup) + 'static,
) -> DropVolumes {
    let locations = query.locations().collect::<Vec<_>>();
    DropVolumes::Pending(PendingVolumeLookup::start(&locations, Box::new(on_ready)))
}

fn source_volume_directory(source: &Location, mounts: &MountTable) -> Location {
    match source.native_path() {
        Some(path) if mounts.is_mount_point(path) => source.clone(),
        _ => source.parent().unwrap_or_else(|| source.clone()),
    }
}

pub(crate) struct PendingVolumeLookup {
    cancellable: gio::Cancellable,
    state: Rc<RefCell<PendingState>>,
}

struct PendingState {
    identities: Vec<Option<VolumeIdentity>>,
    remaining: usize,
    on_ready: Option<Box<dyn FnOnce(DropVolumeLookup)>>,
    waiting: Vec<Box<dyn FnOnce(VolumeRelation)>>,
}

impl PendingVolumeLookup {
    fn start(locations: &[&Location], on_ready: Box<dyn FnOnce(DropVolumeLookup)>) -> Self {
        let cancellable = gio::Cancellable::new();
        let state = Rc::new(RefCell::new(PendingState {
            identities: vec![None; locations.len()],
            remaining: locations.len(),
            on_ready: Some(on_ready),
            waiting: Vec::new(),
        }));
        for (index, location) in locations.iter().enumerate() {
            let file = gio_file_for_location(location);
            let backend = file.uri_scheme().unwrap_or_default().to_string();
            let state = state.clone();
            file.query_info_async(
                gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
                Some(&cancellable),
                move |result| {
                    let identity = result
                        .ok()
                        .and_then(|info| identity_from_gio(&info, &backend));
                    PendingState::resolve(&state, index, identity);
                },
            );
        }
        // Cancelling does not unblock a stat already stuck in the kernel on a
        // dead mount, so the timeout finishes the lookup with what it has.
        let cancel = cancellable.clone();
        let timed_out = state.clone();
        glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(REMOTE_QUERY_TIMEOUT).await;
            cancel.cancel();
            PendingState::finish(&timed_out);
        });
        Self { cancellable, state }
    }

    pub(crate) fn wait(&self, then: impl FnOnce(VolumeRelation) + 'static) {
        self.state.borrow_mut().waiting.push(Box::new(then));
    }

    pub(crate) fn cancel(&self) {
        let mut state = self.state.borrow_mut();
        state.on_ready = None;
        if state.waiting.is_empty() {
            self.cancellable.cancel();
        }
    }
}

impl Drop for PendingVolumeLookup {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl PendingState {
    fn resolve(state: &Rc<RefCell<Self>>, index: usize, identity: Option<VolumeIdentity>) {
        {
            let mut state = state.borrow_mut();
            if state.on_ready.is_none() && state.waiting.is_empty() {
                return;
            }
            state.identities[index] = identity;
            state.remaining -= 1;
            if state.remaining > 0 {
                return;
            }
        }
        Self::finish(state);
    }

    fn finish(state: &Rc<RefCell<Self>>) {
        let (on_ready, waiting, lookup) = {
            let mut state = state.borrow_mut();
            let on_ready = state.on_ready.take();
            let waiting = std::mem::take(&mut state.waiting);
            if on_ready.is_none() && waiting.is_empty() {
                return;
            }
            let identities = std::mem::take(&mut state.identities);
            (
                on_ready,
                waiting,
                DropVolumeLookup::from_identities(identities),
            )
        };
        let relation = lookup.relation;
        if let Some(on_ready) = on_ready {
            on_ready(lookup);
        }
        for then in waiting {
            then(relation);
        }
    }
}

pub(crate) fn volume_identity_of_existing_ancestor(location: &Location) -> Option<VolumeIdentity> {
    volume_identity_of_existing_ancestor_with(location, gio::FileQueryInfoFlags::NONE)
}

fn volume_identity_of_existing_ancestor_with(
    location: &Location,
    flags: gio::FileQueryInfoFlags,
) -> Option<VolumeIdentity> {
    let mut current = location.clone();
    loop {
        if let Some(identity) = native_volume_identity_with(&current, flags) {
            return Some(identity);
        }
        current = current.parent()?;
    }
}

pub(crate) fn restore_volume_relation(source: &Location, dest: &Location) -> VolumeRelation {
    volume_relation(
        volume_identity_of_existing_ancestor(dest).as_ref(),
        &[volume_identity_of_existing_ancestor_with(
            source,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        )],
    )
}

/// Uses GIO for native paths so native and `file://` identities are comparable.
fn native_volume_identity_with(
    location: &Location,
    flags: gio::FileQueryInfoFlags,
) -> Option<VolumeIdentity> {
    let info = gio::File::for_path(location.native_path()?)
        .query_info(
            gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
            flags,
            None::<&gio::Cancellable>,
        )
        .ok()?;
    identity_from_gio(&info, "file")
}

fn identity_from_gio(info: &gio::FileInfo, backend: &str) -> Option<VolumeIdentity> {
    let filesystem_id = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM)?;
    if filesystem_id.is_empty() {
        return None;
    }
    Some(VolumeIdentity {
        filesystem_id: filesystem_id.to_string(),
        backend: backend.into(),
    })
}

fn drive_can_unplug(drive: &gio::Drive) -> bool {
    drive.is_removable() || drive.is_media_removable() || drive.can_eject()
}

pub(crate) fn mount_can_unplug(mount: &gio::Mount) -> bool {
    if mount.can_eject() {
        return true;
    }
    mount
        .drive()
        .or_else(|| mount.volume().and_then(|volume| volume.drive()))
        .is_some_and(|drive| drive_can_unplug(&drive))
}

pub(crate) fn volume_can_unplug(volume: &gio::Volume) -> bool {
    if volume.can_eject() {
        return true;
    }
    volume.drive().is_some_and(|drive| drive_can_unplug(&drive))
        || volume.get_mount().is_some_and(|mount| mount.can_eject())
}

pub(crate) fn mount_is_removable(mount: &gio::Mount) -> bool {
    mount_can_unplug(mount)
        || mount
            .volume()
            .is_some_and(|volume| volume_can_unplug(&volume))
}
