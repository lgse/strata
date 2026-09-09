// SPDX-License-Identifier: GPL-3.0-or-later

mod mounts;
#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    collections::HashSet,
    path::{Component, Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::gio_file_for_location;
use crate::{
    model::Location,
    services::{VolumeIdentity, VolumeRelation, volume_relation},
};

pub(crate) use mounts::MountTable;

pub(crate) const REMOTE_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DropVolumeQuery {
    pub dest: Location,
    pub source_parents: Vec<Location>,
}

impl DropVolumeQuery {
    pub(crate) fn new(dest: &Location, sources: &[Location]) -> Self {
        Self::with_mounts(dest, sources, &MountTable::current())
    }

    fn with_mounts(dest: &Location, sources: &[Location], mounts: &MountTable) -> Self {
        let mut seen = HashSet::new();
        let source_parents = sources
            .iter()
            .map(|source| source_volume_directory(source, mounts))
            .filter(|parent| seen.insert(parent.clone()))
            .collect();
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
    lookup_drop_volumes_with_mounts(query, &MountTable::current(), on_ready)
}

fn lookup_drop_volumes_with_mounts(
    query: &DropVolumeQuery,
    mounts: &MountTable,
    on_ready: impl FnOnce(DropVolumeLookup) + 'static,
) -> DropVolumes {
    let locations = query
        .locations()
        .map(|location| Directory::classify(location, mounts))
        .collect::<Vec<_>>();
    if locations
        .iter()
        .all(|directory| directory.resolves_synchronously())
    {
        let identities = locations
            .iter()
            .map(|directory| native_volume_identity(directory.location))
            .collect();
        return DropVolumes::Ready(DropVolumeLookup::from_identities(identities));
    }
    DropVolumes::Pending(PendingVolumeLookup::start(&locations, Box::new(on_ready)))
}

struct Directory<'a> {
    location: &'a Location,
    is_remote: bool,
    synchronous: bool,
}

impl<'a> Directory<'a> {
    fn classify(location: &'a Location, mounts: &MountTable) -> Self {
        let is_remote = match location.native_path() {
            Some(path) => mounts.is_remote_path(path),
            None => location_is_remote(location),
        };
        let synchronous = !is_remote
            && location
                .native_path()
                .is_some_and(|path| !native_query_may_leave_mount(path));
        Self {
            location,
            is_remote,
            synchronous,
        }
    }

    fn resolves_synchronously(&self) -> bool {
        self.synchronous
    }
}

fn source_volume_directory(source: &Location, mounts: &MountTable) -> Location {
    match source.native_path() {
        Some(path) if mounts.is_mount_point(path) => source.clone(),
        _ => source.parent().unwrap_or_else(|| source.clone()),
    }
}

/// GIO may follow a local-looking path onto a blocking remote mount.
fn native_query_may_leave_mount(path: &Path) -> bool {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return true;
        }
        prefix.push(component);
        if matches!(component, Component::Normal(_))
            && std::fs::symlink_metadata(&prefix).is_ok_and(|meta| meta.is_symlink())
        {
            return true;
        }
    }
    false
}

pub(crate) struct PendingVolumeLookup {
    cancellable: gio::Cancellable,
    state: Rc<RefCell<PendingState>>,
}

struct PendingState {
    identities: Vec<Option<VolumeIdentity>>,
    remaining: usize,
    on_ready: Option<Box<dyn FnOnce(DropVolumeLookup)>>,
}

impl PendingVolumeLookup {
    fn start(directories: &[Directory<'_>], on_ready: Box<dyn FnOnce(DropVolumeLookup)>) -> Self {
        let cancellable = gio::Cancellable::new();
        let state = Rc::new(RefCell::new(PendingState {
            identities: vec![None; directories.len()],
            remaining: directories.len(),
            on_ready: Some(on_ready),
        }));
        for (index, directory) in directories.iter().enumerate() {
            if directory.resolves_synchronously() {
                let mut state = state.borrow_mut();
                state.identities[index] = native_volume_identity(directory.location);
                state.remaining -= 1;
                continue;
            }
            let is_remote = directory.is_remote;
            let state = state.clone();
            gio_file_for_location(directory.location).query_info_async(
                gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
                Some(&cancellable),
                move |result| {
                    let identity = result
                        .ok()
                        .and_then(|info| identity_from_gio(&info, is_remote));
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

    pub(crate) fn cancel(&self) {
        self.state.borrow_mut().on_ready = None;
        self.cancellable.cancel();
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
            if state.on_ready.is_none() {
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
        let (on_ready, lookup) = {
            let mut state = state.borrow_mut();
            let Some(on_ready) = state.on_ready.take() else {
                return;
            };
            let identities = std::mem::take(&mut state.identities);
            (on_ready, DropVolumeLookup::from_identities(identities))
        };
        on_ready(lookup);
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
pub(crate) fn native_volume_identity(location: &Location) -> Option<VolumeIdentity> {
    native_volume_identity_with(location, gio::FileQueryInfoFlags::NONE)
}

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
    identity_from_gio(&info, false)
}

pub(crate) fn location_is_remote(location: &Location) -> bool {
    match location.native_path() {
        Some(path) => MountTable::current().is_remote_path(path),
        None => {
            let scheme = location.backend_name();
            scheme != "file" && scheme != "trash"
        }
    }
}

fn identity_from_gio(info: &gio::FileInfo, is_remote: bool) -> Option<VolumeIdentity> {
    let filesystem_id = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM)?;
    if filesystem_id.is_empty() {
        return None;
    }
    Some(VolumeIdentity {
        filesystem_id: filesystem_id.to_string(),
        is_remote,
    })
}
