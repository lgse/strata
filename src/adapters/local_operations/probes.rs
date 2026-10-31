// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    io,
    ops::ControlFlow,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
};

use gtk::{gio, glib, prelude::*};
use rustix::{
    fs::{Access, FileType, Mode, OFlags},
    io::Errno,
};

use super::fs_ops::{Entry, folder_stamp, fs, list_folder};
use crate::{
    adapters::volume::{
        mount_lanes::{self, Class, Progress, Unanswered},
        mount_watch,
    },
    model::Location,
    services::{LocationValidationError, file_activity, io_error_detail},
};

#[derive(Debug)]
pub(crate) enum Presence {
    Absent,
    Present,
    Unknown,
}

fn unanswered(unanswered: Unanswered) -> io::Error {
    match unanswered {
        Unanswered::TimedOut | Unanswered::Skipped => io::ErrorKind::TimedOut.into(),
        Unanswered::Failed => {
            io::Error::other(crate::i18n::tr("The folder check stopped unexpectedly"))
        }
    }
}

fn absent(error: Errno) -> bool {
    matches!(error, Errno::NOENT | Errno::NOTDIR)
}

fn presence_of(answer: Result<(), Errno>) -> Presence {
    match answer {
        Ok(()) => Presence::Present,
        Err(error) if absent(error) => Presence::Absent,
        Err(_) => Presence::Unknown,
    }
}

fn open_folder(path: &Path) -> Result<OwnedFd, Errno> {
    fs().open_path(
        path.as_os_str(),
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

type Names = Vec<(usize, Option<OsString>)>;

pub(crate) async fn presence(
    locations: &[Location],
    cancellable: &gio::Cancellable,
) -> Vec<Presence> {
    let mounts = mount_watch::latest();
    let points: HashSet<&Path> = mounts.points().map(|(_, point)| point).collect();
    let mut groups: BTreeMap<Option<u64>, BTreeMap<PathBuf, Names>> = BTreeMap::new();
    let context = glib::MainContext::ref_thread_default();
    let mut remote = Vec::new();
    for (index, location) in locations.iter().enumerate() {
        match location.native_path() {
            Some(path) => {
                let (folder, name) = match (path.parent(), path.file_name()) {
                    (Some(parent), Some(name)) if !points.contains(path) => {
                        (parent, Some(name.to_owned()))
                    }
                    _ => (path, None),
                };
                groups
                    .entry(mounts.mount_for(folder).map(|mount| mount.mount_id))
                    .or_default()
                    .entry(folder.to_owned())
                    .or_default()
                    .push((index, name));
            }
            None => {
                let (location, cancellable) = (location.clone(), cancellable.clone());
                remote.push(context.spawn_local(async move {
                    (index, remote_presence(&location, &cancellable).await)
                }));
            }
        }
    }
    let native: Vec<_> = groups
        .into_values()
        .map(|folders| {
            let indexes: Vec<usize> = folders
                .values()
                .flatten()
                .map(|(index, _)| *index)
                .collect();
            let cancellable = cancellable.clone();
            let found = gio::spawn_blocking(move || {
                folders
                    .into_iter()
                    .flat_map(|(folder, names)| {
                        match mount_lanes::admit(&folder, Class::Interactive) {
                            Ok(admitted) => {
                                names_in(&folder, &names, &cancellable, admitted.progress())
                            }
                            Err(_) => names
                                .iter()
                                .map(|(index, _)| (*index, Presence::Unknown))
                                .collect(),
                        }
                    })
                    .collect::<Vec<_>>()
            });
            async move {
                found.await.unwrap_or_else(|_| {
                    indexes
                        .into_iter()
                        .map(|index| (index, Presence::Unknown))
                        .collect()
                })
            }
        })
        .collect();
    let mut answers: Vec<Option<Presence>> = locations.iter().map(|_| None).collect();
    for found in native {
        for (index, presence) in found.await {
            answers[index] = Some(presence);
        }
    }
    for found in remote {
        if let Ok((index, presence)) = found.await {
            answers[index] = Some(presence);
        }
    }
    answers
        .into_iter()
        .map(|answer| answer.unwrap_or(Presence::Unknown))
        .collect()
}

fn names_in(
    folder: &Path,
    names: &[(usize, Option<OsString>)],
    cancellable: &gio::Cancellable,
    progress: &Progress,
) -> Vec<(usize, Presence)> {
    let mut dir = None;
    names
        .iter()
        .map(|(index, name)| {
            if cancellable.is_cancelled() {
                return (*index, Presence::Unknown);
            }
            let Some(name) = name else {
                let stamp = folder_stamp(folder, false);
                progress.answered(&stamp);
                return (*index, presence_of(stamp.map(drop)));
            };
            let dir = dir.get_or_insert_with(|| {
                let opened = open_folder(folder);
                progress.answered(&opened);
                opened
            });
            let presence = match dir {
                Ok(dir) => {
                    let stat = fs().stat(Entry::new(dir, name), false);
                    progress.answered(&stat);
                    presence_of(stat.map(drop))
                }
                Err(error) => presence_of(Err(*error)),
            };
            (*index, presence)
        })
        .collect()
}

async fn remote_presence(location: &Location, cancellable: &gio::Cancellable) -> Presence {
    let Some(uri) = location.uri_value() else {
        return Presence::Unknown;
    };
    let file = gio::File::for_uri(uri);
    let query = gio::CancellableFuture::new(
        file.query_info_future(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        ),
        cancellable.clone(),
    );
    match mount_lanes::run_remote(mount_lanes::authority(uri), Class::Interactive, query).await {
        Ok(Ok(Ok(_))) => Presence::Present,
        Ok(Ok(Err(error))) if error.matches(gio::IOErrorEnum::NotFound) => Presence::Absent,
        _ => Presence::Unknown,
    }
}

fn listable(folder: &Path, progress: &Progress) -> Result<(), LocationValidationError> {
    let invalid = |error: Errno| LocationValidationError::from(io::Error::from(error));
    let stamp = folder_stamp(folder, false);
    progress.answered(&stamp);
    let stamp = stamp.map_err(invalid)?;
    if let Some(id) = stamp.mount_id {
        progress.learn(id);
    }
    if stamp.kind != FileType::Directory {
        return Err(LocationValidationError::NotDirectory);
    }
    let opened = fs().open_path(
        folder.as_os_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    );
    progress.answered(&opened);
    opened.map(drop).map_err(invalid)
}

pub(crate) fn unavailable(refused: Unanswered) -> LocationValidationError {
    LocationValidationError::Unavailable(io_error_detail(&unanswered(refused)))
}

pub(crate) async fn folder(path: &Path) -> Result<(), LocationValidationError> {
    let folder = path.to_owned();
    mount_lanes::run_joined(path, "folder check".to_owned(), move |progress, _| {
        progress.begin_reading();
        listable(&folder, progress)
    })
    .await
    .map_or_else(|refused| Err(unavailable(refused)), Arc::unwrap_or_clone)
}

pub(crate) async fn real_folder(path: &Path) -> Result<PathBuf, LocationValidationError> {
    let folder = path.to_owned();
    mount_lanes::run_joined(path, "real folder check".to_owned(), move |progress, _| {
        progress.begin_reading();
        let real = folder.canonicalize();
        progress.answered(&real);
        let real = real?;
        listable(&real, progress)?;
        Ok(real)
    })
    .await
    .map_or_else(|refused| Err(unavailable(refused)), Arc::unwrap_or_clone)
}

pub(crate) async fn writable_folder(path: &Path) -> Result<Option<PathBuf>, Unanswered> {
    let folder = path.to_owned();
    mount_lanes::run_joined(path, "writable folder".to_owned(), move |progress, _| {
        progress.begin_reading();
        let stamp = folder_stamp(&folder, false);
        progress.answered(&stamp);
        let stamp = stamp.ok()?;
        if let Some(id) = stamp.mount_id {
            progress.learn(id);
        }
        if stamp.kind != FileType::Directory {
            return None;
        }
        let writable = fs().access(&folder, Access::WRITE_OK);
        progress.answered(&writable);
        writable.ok()?;
        folder.canonicalize().ok()
    })
    .await
    .map(Arc::unwrap_or_clone)
}

pub(crate) async fn folder_was_deleted(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    if mount_watch::latest().has_mount_inside(parent) {
        return false;
    }
    let (folder, name) = (parent.to_owned(), name.to_owned());
    let kind = format!("whether {name:?} was deleted");
    mount_lanes::run_joined(parent, kind, move |progress, _| {
        progress.begin_reading();
        let opened = open_folder(&folder);
        progress.answered(&opened);
        let Ok(dir) = opened else {
            return false;
        };
        let stat = fs().stat(Entry::new(&dir, &name), false);
        progress.answered(&stat);
        if !stat.is_err_and(|error| error == Errno::NOENT) {
            return false;
        }
        let mut holds_others = false;
        let listed = list_folder(&folder, &mut |chunk| {
            progress.beat();
            holds_others = !chunk.is_empty();
            ControlFlow::Break(())
        });
        progress.answered(&listed);
        listed.is_ok() && holds_others
    })
    .await
    .is_ok_and(|deleted| *deleted)
}

pub(crate) async fn child_folders(
    dir: &Path,
    prefix: &str,
    include_hidden: bool,
    scanned: usize,
) -> Vec<String> {
    let (folder, prefix) = (dir.to_owned(), prefix.to_lowercase());
    let kind = format!("folders named {prefix:?}…, hidden {include_hidden}, among {scanned}");
    mount_lanes::run_joined(dir, kind, move |progress, cancellable| {
        let real = folder.canonicalize().unwrap_or_else(|_| folder.clone());
        let Ok(hold) = file_activity::hold([&folder, &real], || {}) else {
            return Vec::new();
        };
        progress.begin_reading();
        let mut names = Vec::new();
        let mut seen = 0;
        let listed = list_folder(&folder, &mut |chunk| {
            progress.beat();
            if hold.stopped() || cancellable.is_cancelled() {
                return ControlFlow::Break(());
            }
            for (name, kind) in chunk {
                seen += 1;
                if seen > scanned {
                    return ControlFlow::Break(());
                }
                let shown = name.to_string_lossy();
                if !include_hidden && shown.starts_with('.')
                    || !shown.to_lowercase().starts_with(&prefix)
                {
                    continue;
                }
                let is_folder = match kind {
                    FileType::Directory => true,
                    FileType::Symlink => {
                        let link = folder.join(&name);
                        progress
                            .reach(&link, None, || folder_stamp(&link, false))
                            .is_some_and(|stamp| {
                                stamp.is_ok_and(|stamp| stamp.kind == FileType::Directory)
                            })
                    }
                    _ => false,
                };
                if is_folder {
                    names.push(shown.into_owned());
                }
            }
            ControlFlow::Continue(())
        });
        progress.answered(&listed);
        names
    })
    .await
    .map_or_else(|_| Vec::new(), Arc::unwrap_or_clone)
}

pub(crate) async fn destination_folder(path: &Path) -> Result<PathBuf, LocationValidationError> {
    let folder = path.to_owned();
    mount_lanes::run_joined(path, "destination folder".to_owned(), move |progress, _| {
        progress.begin_reading();
        let real = folder.canonicalize();
        progress.answered(&real);
        let real = real?;
        let stamp = folder_stamp(&real, false);
        progress.answered(&stamp);
        let stamp = stamp.map_err(|error| LocationValidationError::from(io::Error::from(error)))?;
        if let Some(id) = stamp.mount_id {
            progress.learn(id);
        }
        if stamp.kind != FileType::Directory {
            return Err(LocationValidationError::NotDirectory);
        }
        Ok(real)
    })
    .await
    .map_or_else(
        |refused| {
            Err(LocationValidationError::Unavailable(io_error_detail(
                &unanswered(refused),
            )))
        },
        Arc::unwrap_or_clone,
    )
}
