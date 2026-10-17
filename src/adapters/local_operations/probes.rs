// SPDX-License-Identifier: MIT

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "step 5's call sites move onto these probes")
)]

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
    fs::{FileType, Mode, OFlags},
    io::Errno,
};

use super::fs_ops::{Entry, folder_stamp, fs, list_folder};
use crate::{
    adapters::volume::{
        mount_lanes::{self, Class, Progress, Unanswered},
        mount_watch,
    },
    model::{EntryKind, Location},
    services::{LocationValidationError, file_activity, gio_error_detail, io_error_detail},
};

#[derive(Debug)]
pub(crate) enum Presence {
    Absent,
    Present(EntryKind),
    Unknown(io::Error),
}

fn unanswered(unanswered: Unanswered) -> io::Error {
    match unanswered {
        Unanswered::TimedOut | Unanswered::Skipped => io::ErrorKind::TimedOut.into(),
        Unanswered::Failed => io::Error::other("the worker failed"),
    }
}

fn listed_kind(kind: FileType) -> EntryKind {
    match kind {
        FileType::Directory => EntryKind::Directory,
        FileType::RegularFile => EntryKind::File,
        FileType::Symlink => EntryKind::SymbolicLink,
        _ => EntryKind::Other,
    }
}

fn gio_kind(kind: gio::FileType) -> EntryKind {
    match kind {
        gio::FileType::Directory => EntryKind::Directory,
        gio::FileType::Regular => EntryKind::File,
        gio::FileType::SymbolicLink => EntryKind::SymbolicLink,
        _ => EntryKind::Other,
    }
}

fn absent(error: Errno) -> bool {
    matches!(error, Errno::NOENT | Errno::NOTDIR)
}

fn presence_of(kind: Result<FileType, Errno>) -> Presence {
    match kind {
        Ok(kind) => Presence::Present(listed_kind(kind)),
        Err(error) if absent(error) => Presence::Absent,
        Err(error) => Presence::Unknown(error.into()),
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

/// Never follows a final symbolic link.
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
                            Err(refused) => names
                                .iter()
                                .map(|(index, _)| (*index, Presence::Unknown(unanswered(refused))))
                                .collect(),
                        }
                    })
                    .collect::<Vec<_>>()
            });
            async move {
                found.await.unwrap_or_else(|_| {
                    indexes
                        .into_iter()
                        .map(|index| (index, Presence::Unknown(unanswered(Unanswered::Failed))))
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
        .map(|answer| answer.unwrap_or_else(|| Presence::Unknown(unanswered(Unanswered::Failed))))
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
                return (*index, Presence::Unknown(io::ErrorKind::Interrupted.into()));
            }
            let Some(name) = name else {
                let stamp = folder_stamp(folder, false);
                progress.answered(&stamp);
                return (*index, presence_of(stamp.map(|stamp| stamp.kind)));
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
                    presence_of(stat.map(|stat| stat.kind))
                }
                Err(error) => presence_of(Err(*error)),
            };
            (*index, presence)
        })
        .collect()
}

fn authority(uri: &str) -> String {
    match glib::Uri::parse(uri, glib::UriFlags::NONE) {
        Ok(parsed) => format!(
            "{}://{}@{}:{}",
            parsed.scheme(),
            parsed.userinfo().unwrap_or_default(),
            parsed.host().unwrap_or_default(),
            parsed.port()
        ),
        Err(_) => uri
            .split_once(':')
            .map_or(uri, |(scheme, _)| scheme)
            .to_owned(),
    }
}

async fn remote_presence(location: &Location, cancellable: &gio::Cancellable) -> Presence {
    let Some(uri) = location.uri_value() else {
        return Presence::Unknown(io::ErrorKind::InvalidInput.into());
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
    match mount_lanes::run_remote(authority(uri), Class::Interactive, query).await {
        Err(refused) => Presence::Unknown(unanswered(refused)),
        Ok(Err(_)) => Presence::Unknown(io::ErrorKind::Interrupted.into()),
        Ok(Ok(Ok(info))) => Presence::Present(gio_kind(info.file_type())),
        Ok(Ok(Err(error))) if error.matches(gio::IOErrorEnum::NotFound) => Presence::Absent,
        Ok(Ok(Err(error))) => Presence::Unknown(io::Error::other(gio_error_detail(&error))),
    }
}

pub(crate) async fn folder(path: &Path) -> Result<(), LocationValidationError> {
    let folder = path.to_owned();
    mount_lanes::run_joined(path, "folder check".to_owned(), move |progress, _| {
        progress.begin_reading();
        let invalid = |error: Errno| LocationValidationError::from(io::Error::from(error));
        let stamp = folder_stamp(&folder, false);
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

/// Any doubt keeps the folder.
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
