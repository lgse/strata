// SPDX-License-Identifier: MIT

//! GIO reports `access::can-trash` on any writable FUSE root, so many FUSE
//! mounts only reveal missing Trash support when a trash attempt fails partway.

#[cfg(test)]
pub(super) mod tests;

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    future::Future,
    io::ErrorKind,
    os::{fd::OwnedFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
};

use gtk::{gio, glib, prelude::*};

use super::{
    local_operations::Deadline,
    volume::{MountTable, TrashMountKey as MountKey, is_fuse_fs_type},
};
use crate::services::ItemIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrashFailure {
    /// The location cannot hold a Trash; a confirmed permanent delete is the alternative.
    Unsupported(UnsupportedReason),
    Cancelled,
    NotFound,
    ReadOnly,
    NoSpace,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnsupportedReason {
    /// GIO itself returned `NOT_SUPPORTED`.
    Reported,
    /// A FUSE mount refused a step of the freedesktop trash protocol.
    FuseRefused,
}

#[derive(Debug)]
pub(crate) struct TrashError {
    pub(crate) failure: TrashFailure,
    pub(crate) error: glib::Error,
}

impl TrashError {
    /// A failed permanent delete has no Trash fallback to offer.
    pub(crate) fn permanent(error: glib::Error) -> Self {
        let failure = if error.matches(gio::IOErrorEnum::Cancelled) {
            TrashFailure::Cancelled
        } else {
            TrashFailure::Other
        };
        Self { failure, error }
    }

    pub(crate) fn is_unsupported(&self) -> bool {
        matches!(self.failure, TrashFailure::Unsupported(_))
    }
}

/// Classifies without the FUSE check.
impl From<glib::Error> for TrashError {
    fn from(error: glib::Error) -> Self {
        Self {
            failure: decide(&error, None, None),
            error,
        }
    }
}

pub(crate) trait TrashEnvironment {
    fn mounts(&self) -> MountTable;
    /// Only stat-ed off the main thread.
    fn home(&self) -> PathBuf;
    fn trash(
        &self,
        file: &gio::File,
        cancellable: &gio::Cancellable,
        deadline: &Deadline,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ItemIdentity>, glib::Error>>>>;
}

pub(crate) struct SystemTrash;

impl TrashEnvironment for SystemTrash {
    fn mounts(&self) -> MountTable {
        super::volume::mount_watch::latest()
    }

    fn home(&self) -> PathBuf {
        glib::home_dir()
    }

    fn trash(
        &self,
        file: &gio::File,
        cancellable: &gio::Cancellable,
        deadline: &Deadline,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ItemIdentity>, glib::Error>>>> {
        let (file, cancellable, deadline) = (file.clone(), cancellable.clone(), deadline.clone());
        Box::pin(async move {
            super::local_operations::trash::trash_entry(&file, &cancellable, &deadline).await
        })
    }
}

/// Trashes one item, records a mount that turned out to lack Trash, and
/// removes the empty trash folders a failed attempt left behind.
pub(crate) async fn trash_with_cleanup(
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<ItemIdentity>, TrashError> {
    trash_with_cleanup_in(&SystemTrash, file, cancellable).await
}

pub(crate) async fn trash_with_cleanup_in(
    environment: &dyn TrashEnvironment,
    file: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<ItemIdentity>, TrashError> {
    let mut leftovers = LeftoverWatch::observe(environment, native_path(file)).await;
    let result = attempt(environment, file, cancellable, &Deadline::default()).await;
    if let Some(path) = native_path(file) {
        leftovers.note(&path, result.as_ref().err().map(|error| error.failure));
    }
    leftovers.clean_up().await;
    result
}

/// Trashes and classifies without cleanup; batch callers own one [`LeftoverWatch`].
pub(crate) async fn attempt(
    environment: &dyn TrashEnvironment,
    file: &gio::File,
    cancellable: &gio::Cancellable,
    deadline: &Deadline,
) -> Result<Option<ItemIdentity>, TrashError> {
    let error = match environment.trash(file, cancellable, deadline).await {
        Ok(identity) => return Ok(identity),
        Err(error) => error,
    };
    let (failure, mount) = classify(environment, file, &error).await;
    if let (TrashFailure::Unsupported(reason), Some(mount)) = (failure, mount) {
        record_unsupported(mount, reason);
    }
    Err(TrashError { failure, error })
}

/// Blocking counterpart of [`trash_with_cleanup`] for worker threads.
pub(crate) fn verified_with_cleanup(
    path: &Path,
    attempt: impl FnOnce() -> Result<(), glib::Error>,
) -> Result<(), TrashError> {
    let mounts = super::volume::mount_watch::latest();
    let home = glib::home_dir();
    let mount = path
        .parent()
        .and_then(|parent| resolved_mount(&mounts, parent))
        .filter(|mount| is_fuse_fs_type(&mount.fs_type))
        .filter(|_| {
            let parent_device = path
                .parent()
                .and_then(|parent| std::fs::metadata(parent).ok())
                .map(|metadata| metadata.dev());
            let home_device = std::fs::metadata(&home).ok().map(|metadata| metadata.dev());
            parent_device.is_some() && parent_device != home_device
        });
    let snapshot = mount
        .as_ref()
        .map(|mount| TrashDirSnapshot::take(&mount.mount_point));
    let error = match attempt() {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let facts = mount
        .as_ref()
        .map(|mount| FuseFacts::gather(path, &mount.mount_point, &home));
    let failure = decide(
        &error,
        mount.as_ref().map(|mount| mount.fs_type.as_str()),
        facts,
    );
    if let (TrashFailure::Unsupported(reason), Some(mount)) = (failure, mount) {
        record_unsupported(mount, reason);
        if let Some(snapshot) = snapshot {
            snapshot.remove_created();
        }
    }
    Err(TrashError { failure, error })
}

fn native_path(file: &gio::File) -> Option<PathBuf> {
    file.has_uri_scheme("file").then(|| file.path()).flatten()
}

async fn classify(
    environment: &dyn TrashEnvironment,
    file: &gio::File,
    error: &glib::Error,
) -> (TrashFailure, Option<MountKey>) {
    let class = error_class(error);
    let needs_mount = matches!(
        class,
        ErrorClass::Refusal | ErrorClass::Settled(TrashFailure::Unsupported(_))
    );
    let Some(path) = native_path(file).filter(|_| needs_mount) else {
        return (decide(error, None, None), None);
    };
    let mounts = environment.mounts();
    let home = environment.home();
    let probed = gio::spawn_blocking(move || {
        let mount = resolved_mount(&mounts, path.parent()?)
            .filter(|mount| is_fuse_fs_type(&mount.fs_type))?;
        let facts = FuseFacts::gather(&path, &mount.mount_point, &home);
        // A failure in the home Trash says nothing about the rest of the mount.
        if facts.same_device_as_home {
            return None;
        }
        Some((mount, matches!(class, ErrorClass::Refusal).then_some(facts)))
    })
    .await
    .ok()
    .flatten();
    let (mount, facts) = probed.map_or((None, None), |(mount, facts)| (Some(mount), facts));
    let fs_type = mount.as_ref().map(|mount| mount.fs_type.as_str());
    (decide(error, fs_type, facts), mount)
}

/// Blocking: follows symlinks so an alias reaches the real mount.
fn resolved_mount(mounts: &MountTable, directory: &Path) -> Option<MountKey> {
    mounts.mount_for(&std::fs::canonicalize(directory).ok()?)
}

/// What a quick, non-creating look at the mount says about a trash refusal.
#[derive(Clone, Copy, Debug)]
pub(super) struct FuseFacts {
    /// GIO used the home Trash, so the failure isn't about this volume.
    pub(super) same_device_as_home: bool,
    /// The item is still in place, so no rename happened.
    pub(super) item_present: bool,
    /// A disconnected mount fails here with `ENOTCONN` or `EIO`.
    pub(super) mount_reachable: bool,
    /// A permanent delete is a real alternative, not another permission error.
    pub(super) parent_writable: bool,
}

impl FuseFacts {
    fn gather(path: &Path, mount_point: &Path, home: &Path) -> Self {
        let parent = path
            .parent()
            .and_then(|parent| std::fs::metadata(parent).ok());
        let item = std::fs::symlink_metadata(path).ok();
        let home_device = std::fs::metadata(home).ok().map(|metadata| metadata.dev());
        Self {
            same_device_as_home: parent
                .as_ref()
                .is_some_and(|parent| Some(parent.dev()) == home_device),
            item_present: item.is_some(),
            mount_reachable: std::fs::metadata(mount_point).is_ok(),
            parent_writable: path.parent().is_some_and(|directory| {
                parent
                    .as_ref()
                    .zip(item.as_ref())
                    .is_some_and(|(parent, item)| may_remove_from(parent, item))
                    && rustix::fs::access(directory, rustix::fs::Access::WRITE_OK).is_ok()
            }),
        }
    }

    fn confirm_refusal(self) -> bool {
        !self.same_device_as_home
            && self.item_present
            && self.mount_reachable
            && self.parent_writable
    }
}

/// FUSE mounts without `default_permissions` often answer `access()` with
/// success regardless (sshfs only checks `X_OK`), so read the mode bits too.
fn may_remove_from(parent: &std::fs::Metadata, item: &std::fs::Metadata) -> bool {
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    let mode = parent.mode();
    let in_group = parent.gid() == gid
        || rustix::process::getgroups()
            .is_ok_and(|groups| groups.iter().any(|group| group.as_raw() == parent.gid()));
    let writable = uid == 0
        || if parent.uid() == uid {
            mode & 0o200 != 0
        } else if in_group {
            mode & 0o020 != 0
        } else {
            mode & 0o002 != 0
        };
    let sticky_allows = mode & 0o1000 == 0 || uid == 0 || parent.uid() == uid || item.uid() == uid;
    writable && sticky_allows
}

#[derive(Clone, Copy)]
enum ErrorClass {
    Settled(TrashFailure),
    /// Errors a FUSE backend returns when it lacks a step of the trash protocol.
    Refusal,
    Other,
}

fn error_class(error: &glib::Error) -> ErrorClass {
    use gio::IOErrorEnum as Io;
    use glib::FileError as File;
    if let Some(code) = error.kind::<Io>() {
        return match code {
            Io::Cancelled => ErrorClass::Settled(TrashFailure::Cancelled),
            Io::NotSupported => {
                ErrorClass::Settled(TrashFailure::Unsupported(UnsupportedReason::Reported))
            }
            Io::NotFound => ErrorClass::Settled(TrashFailure::NotFound),
            Io::ReadOnly => ErrorClass::Settled(TrashFailure::ReadOnly),
            Io::NoSpace => ErrorClass::Settled(TrashFailure::NoSpace),
            Io::PermissionDenied | Io::Failed | Io::InvalidArgument => ErrorClass::Refusal,
            _ => ErrorClass::Other,
        };
    }
    // GLib writes the `.trashinfo` file with g_file_set_contents, which reports G_FILE_ERROR.
    match error.kind::<File>() {
        Some(File::Nospc) => ErrorClass::Settled(TrashFailure::NoSpace),
        Some(File::Noent) => ErrorClass::Settled(TrashFailure::NotFound),
        Some(File::Rofs) => ErrorClass::Settled(TrashFailure::ReadOnly),
        Some(File::Nametoolong) | None => ErrorClass::Other,
        Some(_) => ErrorClass::Refusal,
    }
}

/// `facts` is `None` when the item isn't native or the mount isn't FUSE.
pub(super) fn decide(
    error: &glib::Error,
    fs_type: Option<&str>,
    facts: Option<FuseFacts>,
) -> TrashFailure {
    match error_class(error) {
        ErrorClass::Settled(failure) => failure,
        ErrorClass::Refusal
            if fs_type.is_some_and(is_fuse_fs_type)
                && facts.is_some_and(FuseFacts::confirm_refusal) =>
        {
            TrashFailure::Unsupported(UnsupportedReason::FuseRefused)
        }
        ErrorClass::Refusal | ErrorClass::Other => TrashFailure::Other,
    }
}

/// Process-wide and in memory: both windows share it, and a remount gets a new key.
static UNSUPPORTED: LazyLock<Mutex<HashMap<MountKey, UnsupportedReason>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn unsupported_mounts() -> MutexGuard<'static, HashMap<MountKey, UnsupportedReason>> {
    UNSUPPORTED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Only FUSE mounts are recorded; GIO's own probe is accurate elsewhere.
pub(crate) fn record_unsupported(mount: MountKey, reason: UnsupportedReason) {
    if is_fuse_fs_type(&mount.fs_type) {
        unsupported_mounts().insert(mount, reason);
    }
}

/// Lexical, so it never touches the mount; an alias through a symlink misses.
pub(crate) fn unsupported_reason_for(path: &Path) -> Option<(MountKey, UnsupportedReason)> {
    if unsupported_mounts().is_empty() {
        return None;
    }
    unsupported_reason_in(&super::volume::mount_watch::latest(), path)
}

/// Blocking: resolves symlinks first, for worker threads already reading `directory`.
pub(crate) fn unsupported_reason_for_directory(directory: &Path) -> Option<UnsupportedReason> {
    if unsupported_mounts().is_empty() {
        return None;
    }
    let directory = std::fs::canonicalize(directory).ok()?;
    unsupported_reason_in(&super::volume::mount_watch::latest(), &directory)
        .map(|(_, reason)| reason)
}

pub(super) fn unsupported_reason_in(
    mounts: &MountTable,
    path: &Path,
) -> Option<(MountKey, UnsupportedReason)> {
    let mount = mounts.mount_for(path)?;
    let reason = *unsupported_mounts().get(&mount)?;
    Some((mount, reason))
}

/// Lets Reload retry Trash on mounts the user may have fixed. Clears every
/// mount so it never has to resolve a possibly hung folder first.
pub(crate) fn forget_all() {
    unsupported_mounts().clear();
}

/// Watches the FUSE mounts a batch trashes on and, when every attempt on a
/// mount was unsupported, removes the trash folders that weren't there before.
pub(crate) struct LeftoverWatch {
    parents: HashMap<PathBuf, MountKey>,
    watched: HashMap<MountKey, WatchedMount>,
}

struct WatchedMount {
    snapshot: TrashDirSnapshot,
    unsupported: bool,
    trashed: bool,
}

impl LeftoverWatch {
    pub(crate) async fn observe(
        environment: &dyn TrashEnvironment,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let mounts = environment.mounts();
        let mut parents: Vec<PathBuf> = paths
            .into_iter()
            .filter_map(|path| path.parent().map(Path::to_owned))
            .collect();
        parents.sort();
        parents.dedup();
        if parents.is_empty() {
            return Self {
                parents: HashMap::new(),
                watched: HashMap::new(),
            };
        }
        let home = environment.home();
        gio::spawn_blocking(move || {
            let home_device = std::fs::metadata(home).ok().map(|metadata| metadata.dev());
            let mut watch = Self {
                parents: HashMap::new(),
                watched: HashMap::new(),
            };
            for parent in parents {
                let Some(mount) = resolved_mount(&mounts, &parent)
                    .filter(|mount| is_fuse_fs_type(&mount.fs_type))
                else {
                    continue;
                };
                let on_home = std::fs::metadata(&parent)
                    .map_or(true, |metadata| Some(metadata.dev()) == home_device);
                if on_home {
                    continue;
                }
                watch
                    .watched
                    .entry(mount.clone())
                    .or_insert_with(|| WatchedMount {
                        snapshot: TrashDirSnapshot::take(&mount.mount_point),
                        unsupported: false,
                        trashed: false,
                    });
                watch.parents.insert(parent, mount);
            }
            watch
        })
        .await
        .unwrap_or_else(|_| Self {
            parents: HashMap::new(),
            watched: HashMap::new(),
        })
    }

    /// `failure` is `None` for a successful trash.
    pub(crate) fn note(&mut self, path: &Path, failure: Option<TrashFailure>) {
        let Some(watched) = path
            .parent()
            .and_then(|parent| self.parents.get(parent))
            .and_then(|mount| self.watched.get_mut(mount))
        else {
            return;
        };
        match failure {
            None => watched.trashed = true,
            Some(TrashFailure::Unsupported(_)) => watched.unsupported = true,
            Some(_) => {}
        }
    }

    pub(crate) async fn clean_up(self) {
        let snapshots: Vec<_> = self
            .watched
            .into_values()
            .filter(|watched| watched.unsupported && !watched.trashed)
            .map(|watched| watched.snapshot)
            .collect();
        if snapshots.is_empty() {
            return;
        }
        let _ = gio::spawn_blocking(move || {
            for snapshot in snapshots {
                snapshot.remove_created();
            }
        })
        .await;
    }
}

/// The two folders GIO may create at a mount root: `.Trash-$uid`, or
/// `.Trash/$uid` when a sticky `.Trash` exists.
fn trash_dir_candidates() -> [(Option<&'static str>, OsString); 2] {
    let uid = rustix::process::getuid().as_raw().to_string();
    [
        (None, format!(".Trash-{uid}").into()),
        (Some(".Trash"), uid.into()),
    ]
}

pub(super) struct TrashDirSnapshot {
    root: PathBuf,
    existed: [bool; 2],
}

impl TrashDirSnapshot {
    pub(super) fn take(root: &Path) -> Self {
        let existed = trash_dir_candidates().map(|(parent, name)| {
            let mut path = root.to_owned();
            path.extend(parent);
            path.push(name);
            // Only a definite absence lets cleanup claim the folder later.
            !std::fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound)
        });
        Self {
            root: root.to_owned(),
            existed,
        }
    }

    pub(super) fn remove_created(&self) {
        for ((parent, name), existed) in trash_dir_candidates().into_iter().zip(self.existed) {
            if existed {
                continue;
            }
            if let Err(error) = remove_empty_trash_dir(&self.root, parent, &name) {
                tracing::debug!(
                    root = %self.root.display(),
                    name = %name.to_string_lossy(),
                    error,
                    "left trash folder in place"
                );
            }
        }
    }
}

/// Never recursive: removes only empty `files`/`info` and then the folder itself.
fn remove_empty_trash_dir(root: &Path, parent: Option<&str>, name: &OsStr) -> Result<(), String> {
    use rustix::{
        fs::{AtFlags, Mode, OFlags},
        io::Errno,
    };
    let root = rustix::fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let parent = match parent {
        Some(parent) => {
            let parent = OsStr::new(parent);
            match super::local_operations::open_local_child_directory(&root, parent) {
                Ok(parent) => parent,
                Err(_)
                    if rustix::fs::statat(&root, parent, AtFlags::SYMLINK_NOFOLLOW)
                        .is_err_and(|error| error == Errno::NOENT) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        None => root,
    };
    let candidate = match super::local_operations::open_local_child_directory(&parent, name) {
        Ok(candidate) => candidate,
        Err(_)
            if rustix::fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)
                .is_err_and(|error| error == Errno::NOENT) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let children = ["files", "info"];
    for child in children {
        match super::local_operations::open_local_child_directory(&candidate, OsStr::new(child)) {
            Ok(directory) if !directory_is_empty(&directory)? => {
                return Err(format!("{child} is not empty"));
            }
            Ok(_) => {}
            Err(_)
                if rustix::fs::statat(&candidate, child, AtFlags::SYMLINK_NOFOLLOW)
                    .is_err_and(|error| error == Errno::NOENT) => {}
            Err(error) => return Err(error),
        }
    }
    for child in children {
        match rustix::fs::unlinkat(&candidate, child, AtFlags::REMOVEDIR) {
            Ok(()) | Err(Errno::NOENT) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    rustix::fs::unlinkat(&parent, name, AtFlags::REMOVEDIR).map_err(|error| error.to_string())
}

fn directory_is_empty(directory: &OwnedFd) -> Result<bool, String> {
    for entry in rustix::fs::Dir::read_from(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_name() != c"." && entry.file_name() != c".." {
            return Ok(false);
        }
    }
    Ok(true)
}
