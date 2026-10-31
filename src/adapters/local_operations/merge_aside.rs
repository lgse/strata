// SPDX-License-Identifier: MIT

//! Where Trash is unavailable, a merge moves each original it overwrites
//! into one hidden folder inside the merge target, under its relative path,
//! and deletes that folder only once the merge has succeeded and is on
//! disk. An original whose name the merge left free goes back to it first.

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    os::fd::{AsFd, OwnedFd},
    path::{Path, PathBuf},
    sync::Arc,
};

use gtk::{gio, glib};
use rustix::{
    fs::{AtFlags, Mode},
    io::Errno,
};

use super::{
    copy_engine::Ownership,
    deletion::{PinnedEntry, parallel_delete_local_with_worker_count},
    fs_ops::{Entry, FsOps, drop_off_main_thread, fs},
    io_error,
    leftovers::{self, Deadline, Refused},
    native_fs::{LocalFileIdentity, Recorded, open_local_parent_directory},
    placement::{self, Feasibility, PlaceError},
    placement_reason,
    progress::TransferProgressTracker,
    run_blocking, was_cancelled,
};
use crate::{
    model::{Location, hidden_names::HiddenName},
    services::{Attributes, KeptItem, TransferReport},
};

struct Folder {
    name: OsString,
    identity: LocalFileIdentity,
    handle: Arc<OwnedFd>,
}

struct Mirror {
    relative: PathBuf,
    parent: Arc<OwnedFd>,
    name: OsString,
    handle: Arc<OwnedFd>,
}

struct Moved {
    path: PathBuf,
    parent: Arc<OwnedFd>,
    aside_parent: Arc<OwnedFd>,
    name: OsString,
    identity: LocalFileIdentity,
    attributes: Option<Attributes>,
}

pub(super) struct MergeAside {
    target_path: PathBuf,
    target: Option<Arc<OwnedFd>>,
    folder: Option<Folder>,
    /// In creation order, so parents precede their children.
    mirrors: Vec<Mirror>,
    moved: Vec<Moved>,
}

struct Kept {
    folder: PathBuf,
    reason: PlaceError,
}

enum Left {
    Nothing,
    StillOpen(KeptItem),
    Something,
}

enum Removed {
    Gone,
    Held(Errno),
    HeldBelow,
    Kept,
}

fn removed(
    fs: &dyn FsOps,
    at: Entry<'_>,
    held: &OwnedFd,
    deadline: &Deadline,
    held_child: impl Fn(&OsStr) -> bool,
) -> Removed {
    if fs.stat(at, false).err() == Some(Errno::NOENT) {
        return Removed::Gone;
    }
    match leftovers::remove_held_folder(fs, at, held.as_fd(), deadline, &|| false) {
        Ok(()) | Err(Refused::Os(Errno::NOENT)) => Removed::Gone,
        Err(Refused::StillOpen(errno)) => Removed::Held(errno),
        Err(Refused::Os(errno @ (Errno::NOTEMPTY | Errno::EXIST))) => {
            let Ok(names) = fs.names(Entry::new(held, OsStr::new("."))) else {
                return Removed::Kept;
            };
            if names.is_empty()
                || names.iter().any(|name| {
                    !held_child(name) && !leftovers::is_leftover(fs, held.as_fd(), name)
                })
            {
                Removed::Kept
            } else if names.iter().any(|name| held_child(name)) {
                Removed::HeldBelow
            } else {
                Removed::Held(errno)
            }
        }
        Err(_) => Removed::Kept,
    }
}

fn could_not_set_aside(name: &str, error: PlaceError) -> glib::Error {
    io_error(rust_i18n::t!(
        "Could not move “%{name}” out of the way before merging: %{error}",
        name = name,
        error = crate::services::error_detail(placement_reason(error))
    ))
}

fn create_folder(target: &OwnedFd) -> Result<Folder, PlaceError> {
    let (name, identity) = placement::reserve_hidden_name(HiddenName::Merge, |name| {
        let entry = Entry::new(target, name);
        fs().mkdir(entry, Mode::from_raw_mode(0o700))
            .map_err(placement::creation_error)?;
        fs().stat(entry, false)
            .map(|stat| stat.identity)
            .map_err(PlaceError::Io)
    })
    .map_err(|error| match error {
        PlaceError::InvalidName => PlaceError::Unsupported,
        error => error,
    })?;
    let opened = placement::open_beneath(fs(), target.as_fd(), Path::new(&name)).and_then(
        |handle| match fs().stat_fd(handle.as_fd()) {
            Ok(stat) if stat.identity == identity => Ok(handle),
            Ok(_) => Err(PlaceError::Changed),
            Err(error) => Err(PlaceError::Io(error)),
        },
    );
    match opened {
        Ok(handle) => Ok(Folder {
            name,
            identity,
            handle: Arc::new(handle),
        }),
        Err(error) => {
            placement::remove_if_unchanged(
                fs(),
                Entry::new(target, &name),
                identity,
                None,
                AtFlags::REMOVEDIR,
            );
            Err(error)
        }
    }
}

impl MergeAside {
    fn new(target_path: PathBuf) -> Self {
        Self {
            target_path,
            target: None,
            folder: None,
            mirrors: Vec::new(),
            moved: Vec::new(),
        }
    }

    fn used(&self) -> bool {
        self.folder.is_some()
    }

    pub(super) fn paths(&self) -> HashSet<PathBuf> {
        self.moved.iter().map(|moved| moved.path.clone()).collect()
    }

    fn folder_path(&self) -> Option<PathBuf> {
        self.folder
            .as_ref()
            .map(|folder| self.target_path.join(&folder.name))
    }

    fn target(&mut self) -> Result<Arc<OwnedFd>, glib::Error> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }
        let target = Arc::new(open_local_parent_directory(&self.target_path).map_err(io_error)?);
        self.target = Some(target.clone());
        Ok(target)
    }

    fn set_aside(&mut self, path: &Path, name: &str) -> Result<(), glib::Error> {
        let failed = |error| could_not_set_aside(name, error);
        let (Some(relative), Some(file_name)) = (
            path.strip_prefix(&self.target_path)
                .ok()
                .and_then(Path::parent),
            path.file_name(),
        ) else {
            return Err(failed(PlaceError::Changed));
        };
        let target = self.target()?;
        if self.folder.is_none() {
            match placement::can_set_aside(fs(), target.as_fd(), false) {
                Ok(Feasibility::Yes) => {}
                Ok(Feasibility::No) => return Err(failed(PlaceError::Unsupported)),
                Err(error) => return Err(failed(error)),
            }
            self.folder = Some(create_folder(&target).map_err(failed)?);
        }
        let parent = if relative.as_os_str().is_empty() {
            target
        } else {
            Arc::new(placement::open_beneath(fs(), target.as_fd(), relative).map_err(failed)?)
        };
        let aside_parent = self.mirror(relative).map_err(failed)?;
        let entry = Entry::new(&*parent, file_name);
        let original = fs()
            .stat(entry, false)
            .map_err(|error| failed(PlaceError::Io(error)))?;
        if original.is_directory() {
            return Err(failed(PlaceError::Changed));
        }
        let (identity, attributes) = placement::move_aside(
            fs(),
            entry,
            Entry::new(&*aside_parent, file_name),
            original.identity,
            original.kept_attributes(),
        )
        .map_err(failed)?;
        self.moved.push(Moved {
            path: path.to_path_buf(),
            parent,
            aside_parent,
            name: file_name.to_os_string(),
            identity,
            attributes,
        });
        Ok(())
    }

    fn mirror(&mut self, relative: &Path) -> Result<Arc<OwnedFd>, PlaceError> {
        let mut current = self
            .folder
            .as_ref()
            .map(|folder| folder.handle.clone())
            .ok_or(PlaceError::Changed)?;
        let mut walked = PathBuf::new();
        for component in relative.iter() {
            walked.push(component);
            if let Some(mirror) = self.mirrors.iter().find(|mirror| mirror.relative == walked) {
                current = mirror.handle.clone();
                continue;
            }
            let entry = Entry::new(&*current, component);
            fs().mkdir(entry, Mode::from_raw_mode(0o700))
                .map_err(placement::creation_error)?;
            let handle = match placement::open_beneath(fs(), current.as_fd(), Path::new(component))
            {
                Ok(handle) => Arc::new(handle),
                Err(error) => {
                    let _ = fs().unlink(entry, AtFlags::REMOVEDIR);
                    return Err(error);
                }
            };
            self.mirrors.push(Mirror {
                relative: walked.clone(),
                parent: current,
                name: component.to_os_string(),
                handle: handle.clone(),
            });
            current = handle;
        }
        Ok(current)
    }

    fn remove_folders(&mut self, deadline: &Deadline) -> Left {
        self.moved.clear();
        let fs = fs();
        let mut held = HashSet::new();
        let mut first = None;
        for mirror in self.mirrors.drain(..).rev() {
            let at = Entry::new(&*mirror.parent, &mirror.name);
            let held_child = |name: &OsStr| held.contains(&mirror.relative.join(name));
            match removed(fs, at, &mirror.handle, deadline, held_child) {
                Removed::Held(errno) => {
                    first.get_or_insert((mirror.parent, mirror.name, errno));
                }
                Removed::HeldBelow => {}
                Removed::Gone | Removed::Kept => continue,
            }
            held.insert(mirror.relative);
        }
        let (Some(target), Some(folder)) = (&self.target, &self.folder) else {
            return Left::Nothing;
        };
        let container = Entry::new(&**target, &folder.name);
        let held_child = |name: &OsStr| held.contains(Path::new(name));
        match removed(fs, container, &folder.handle, deadline, held_child) {
            Removed::Gone => return Left::Nothing,
            Removed::Kept => return Left::Something,
            Removed::Held(errno) => {
                first.get_or_insert_with(|| (Arc::clone(target), folder.name.clone(), errno));
            }
            Removed::HeldBelow => {}
        }
        let Some((dir, name, errno)) = first else {
            return Left::Something;
        };
        Left::StillOpen(KeptItem {
            location: Location::local(self.target_path.join(&folder.name)),
            cause: leftovers::kept_open(fs, Entry::new(&*dir, &name), errno),
        })
    }

    fn put_back(&mut self, placed: &HashMap<PathBuf, Ownership>) -> Result<Option<KeptItem>, Kept> {
        let mut failure = None;
        for moved in std::mem::take(&mut self.moved).into_iter().rev() {
            let aside = Entry::new(&*moved.aside_parent, &moved.name);
            let original = Entry::new(&*moved.parent, &moved.name);
            let put_back =
                || placement::place_back(fs(), aside, original, moved.identity, moved.attributes);
            let mut put = put_back();
            if put == Err(PlaceError::Exists)
                && placed
                    .get(&moved.path)
                    .is_some_and(|copy| copy.discard(fs(), original))
            {
                put = put_back();
            }
            if let Err(error) = put {
                failure.get_or_insert(error);
            }
        }
        let reason = match (self.remove_folders(&Deadline::default()), failure) {
            (Left::Nothing, _) => return Ok(None),
            (Left::StillOpen(kept), None) => return Ok(Some(kept)),
            (_, failure) => failure.unwrap_or(PlaceError::Unconfirmed),
        };
        Err(Kept {
            folder: self.folder_path().unwrap_or_default(),
            reason,
        })
    }

    /// Moves each original whose name the merge left free back to it: the
    /// incoming item was left out, or kept at its source by a move.
    fn put_back_unreplaced(&mut self) -> Result<(), PlaceError> {
        let mut failure = Ok(());
        self.moved.retain(|moved| {
            let original = Entry::new(&*moved.parent, &moved.name);
            if fs().stat(original, false).err() != Some(Errno::NOENT) {
                return true;
            }
            let aside = Entry::new(&*moved.aside_parent, &moved.name);
            match placement::place_back(fs(), aside, original, moved.identity, moved.attributes) {
                Ok(_) => false,
                // `Exists` too: a name taken since the check was not replaced by the merge.
                Err(error) => {
                    failure = failure.and(Err(error));
                    true
                }
            }
        });
        failure
    }

    pub(super) async fn remove(
        mut self,
        progress: Option<&TransferProgressTracker>,
        deadline: &Deadline,
    ) -> Result<(), glib::Error> {
        let path = self.folder_path().unwrap_or_default();
        let (Some(target), Some(folder)) = (self.target.clone(), self.folder.as_ref()) else {
            return Ok(());
        };
        let root = PinnedEntry {
            parent: target,
            name: folder.name.clone(),
            identity: Some(Recorded {
                identity: folder.identity,
                attributes: None,
            }),
        };
        let deleted = parallel_delete_local_with_worker_count(
            vec![root],
            gio::Cancellable::new(),
            None,
            deadline.clone(),
        )
        .await;
        let Err(error) = deleted else {
            return Ok(());
        };
        let deadline = deadline.clone();
        let left = run_blocking(move || {
            Ok(match self.remove_folders(&deadline) {
                Left::Nothing => Ok(None),
                Left::StillOpen(kept) => Ok(Some(kept)),
                Left::Something => Err(()),
            })
        })
        .await;
        match (left, progress) {
            (Ok(Ok(None)), _) => Ok(()),
            (Ok(Ok(Some(kept))), Some(progress)) => {
                progress.report(TransferReport {
                    kept: vec![kept],
                    ..TransferReport::default()
                });
                Ok(())
            }
            _ => Err(not_removed(&path, &error)),
        }
    }

    pub(super) fn not_removed(&self, error: &glib::Error) -> glib::Error {
        not_removed(&self.folder_path().unwrap_or_default(), error)
    }
}

impl Drop for MergeAside {
    fn drop(&mut self) {
        drop_off_main_thread((
            self.target.take(),
            self.folder.take(),
            std::mem::take(&mut self.mirrors),
            std::mem::take(&mut self.moved),
        ));
    }
}

fn not_removed(folder: &Path, error: &glib::Error) -> glib::Error {
    io_error(rust_i18n::t!(
        "The folders were merged, but the replaced originals could not be removed from “%{folder}”: %{error}",
        folder = folder.display(),
        error = crate::services::gio_error_detail(error)
    ))
}

pub(super) async fn set_aside(
    aside: &mut Option<MergeAside>,
    root: &Path,
    path: &Path,
    name: String,
) -> Result<(), glib::Error> {
    let mut step = aside
        .take()
        .unwrap_or_else(|| MergeAside::new(root.to_path_buf()));
    let path = path.to_path_buf();
    let (step, result) = run_blocking(move || {
        let result = step.set_aside(&path, &name);
        Ok((step, result))
    })
    .await?;
    *aside = Some(step);
    result
}

pub(super) async fn abandon(
    aside: Option<MergeAside>,
    error: glib::Error,
    placed: HashMap<PathBuf, Ownership>,
    progress: Option<&TransferProgressTracker>,
) -> glib::Error {
    let Some(mut aside) = aside.filter(MergeAside::used) else {
        return error;
    };
    let folder = aside.folder_path().unwrap_or_default();
    let kept = match gio::spawn_blocking(move || aside.put_back(&placed)).await {
        Ok(Ok(open)) => {
            if let Some(progress) = progress {
                progress.report(TransferReport {
                    kept: open.into_iter().collect(),
                    ..TransferReport::default()
                });
            }
            return error;
        }
        Ok(Err(kept)) => kept,
        Err(_) => Kept {
            folder,
            reason: PlaceError::Unconfirmed,
        },
    };
    let reason = if was_cancelled(&error) {
        crate::services::error_detail(placement_reason(kept.reason))
    } else {
        crate::services::gio_error_detail(&error)
    };
    io_error(rust_i18n::t!(
        "The merge stopped. Originals that could not be put back are in “%{folder}”: %{error}",
        folder = kept.folder.display(),
        error = reason
    ))
}

pub(super) async fn finish(aside: Option<MergeAside>) -> Result<Option<MergeAside>, glib::Error> {
    let Some(mut aside) = aside.filter(MergeAside::used) else {
        return Ok(None);
    };
    let (aside, put_back) = run_blocking(move || {
        let put_back = aside.put_back_unreplaced();
        Ok((aside, put_back))
    })
    .await?;
    if let Err(error) = put_back {
        return Err(io_error(rust_i18n::t!(
            "The folders were merged, but the replaced originals could not be removed from “%{folder}”: %{error}",
            folder = aside.folder_path().unwrap_or_default().display(),
            error = crate::services::error_detail(placement_reason(error))
        )));
    }
    Ok(Some(aside))
}
