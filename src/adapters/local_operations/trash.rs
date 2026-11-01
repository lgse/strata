// SPDX-License-Identifier: MIT

//! Trash that only counts a local item as trashed once it has left its name:
//! GLib 2.86 to 2.88.4 can fail without an error and move nothing (GLib #4060).

use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use gtk::{gio, glib, prelude::*};
use rustix::io::Errno;

use super::{
    await_cancellable, cancelled_local_operation,
    deferred_close::{self, WaitCancelled},
    fs_ops::{Entry, EntryStat, fs},
    holders, io_error,
    leftovers::Deadline,
    mount_profile::{identity_at, known_mount},
    native_fs::{
        self, Pinned, identity_at_path, open_local_parent_directory, os_error_text,
        trashed_identity,
    },
    run_blocking, translated_io_error,
};
use crate::{
    adapters::{
        trash_restore::{decode_trashinfo_path, trash_root_from_files_path},
        trash_support::{TrashError, TrashFailure},
        volume::{SillyNames, mount_watch},
    },
    model::hidden_names::ForeignLeftover,
    services::{IdentityTraits, ItemIdentity},
};

const NOT_MOVED_REASON: &str = "Trash reported success, but the item was not moved";

pub(in crate::adapters) async fn trash_entry(
    file: &gio::File,
    cancellable: &gio::Cancellable,
    deadline: &Deadline,
) -> Result<Option<ItemIdentity>, glib::Error> {
    if let Some(path) = file.path().filter(|_| file.is_native()) {
        let (cancellable, deadline) = (cancellable.clone(), deadline.clone());
        return run_blocking(move || trash_local_blocking(&path, &cancellable, &deadline)).await;
    }
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.trash_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
    .map(|()| None)
}

pub(super) fn trash_local_blocking(
    path: &Path,
    cancellable: &gio::Cancellable,
    deadline: &Deadline,
) -> Result<Option<ItemIdentity>, glib::Error> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(errno_error(Errno::INVAL));
    };
    let parent = open_local_parent_directory(parent).map_err(io_error)?;
    let at = Entry::new(&parent, name);
    let stat = fs().stat(at, false).map_err(errno_error)?;
    verify_trash(at, path, &Held::take(at, stat), cancellable, deadline)
}

/// An item still there afterwards fails as `NotSupported`, so callers fall
/// back as they would without Trash.
pub(super) fn trash_verified(
    at: Entry<'_>,
    path: &Path,
    stat: &EntryStat,
    cancellable: &gio::Cancellable,
) -> Result<Option<ItemIdentity>, TrashError> {
    let held = Held::take(at, *stat);
    if !native_fs::unchanged(
        held.traits,
        stat.identity,
        stat.file_attributes(),
        &held.stat,
    ) {
        return Err(TrashError {
            failure: TrashFailure::Other,
            error: translated_io_error("The target changed while the operation was in progress"),
        });
    }
    let mut trashed = None;
    crate::adapters::trash_support::verified_with_cleanup(path, || {
        trashed = verify_trash(at, path, &held, cancellable, &Deadline::default())?;
        Ok(())
    })?;
    Ok(trashed)
}

struct Held {
    stat: EntryStat,
    pin: Pinned,
    traits: IdentityTraits,
}

impl Held {
    fn take(at: Entry<'_>, stat: EntryStat) -> Self {
        let (_, capabilities) = known_mount(fs(), at.dir);
        let traits = capabilities.identity;
        if capabilities.silly_names == Some(SillyNames::Renamed(ForeignLeftover::Nfs)) {
            let pin = Pinned::at(fs(), at, &stat, traits);
            return Self { stat, pin, traits };
        }
        let (stat, pin) = Pinned::hold(fs(), at).unwrap_or_else(|| (stat, Pinned::default()));
        Self { stat, pin, traits }
    }
}

fn verify_trash(
    at: Entry<'_>,
    path: &Path,
    held: &Held,
    cancellable: &gio::Cancellable,
    deadline: &Deadline,
) -> Result<Option<ItemIdentity>, glib::Error> {
    let Held { stat, pin, traits } = held;
    let trashed = deferred_close::retry_folder_rename(
        fs(),
        at,
        deadline,
        &|| cancellable.is_cancelled(),
        |error: &glib::Error| error.matches(gio::IOErrorEnum::PermissionDenied),
        || fs().trash(path, cancellable),
    );
    let trashed = match trashed {
        Ok(Ok(trashed)) => trashed,
        Ok(Err(error)) => return Err(still_open(at, error)),
        Err(WaitCancelled) => return Err(cancelled_local_operation()),
    };
    let still_there = |current: &EntryStat| {
        pin.holds(fs(), current).unwrap_or_else(|| {
            native_fs::unchanged(*traits, stat.identity, stat.kept_attributes(), current)
        })
    };
    if !trashed
        || fs()
            .stat(at, false)
            .is_ok_and(|current| still_there(&current))
    {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr(NOT_MOVED_REASON),
        ));
    }
    Ok(match pin.location() {
        Some(location) if !in_trash(&location) => trashed_instead(path),
        _ => Some(trashed_identity(stat, identity_at(fs(), at.dir))),
    })
}

fn in_trash(location: &Path) -> bool {
    trash_root_from_files_path(location).is_some_and(|root| {
        let name = root.file_name().map(OsStr::as_bytes).unwrap_or_default();
        name == b"Trash"
            || name.starts_with(b".Trash-")
            || root.parent().and_then(Path::file_name) == Some(OsStr::new(".Trash"))
    })
}

fn trashed_instead(path: &Path) -> Option<ItemIdentity> {
    let name = path.file_name()?;
    let uid = rustix::process::getuid().as_raw();
    let mut roots = vec![(glib::user_data_dir().join("Trash"), None)];
    if let Some(mount) = mount_watch::latest().mount_point_for(path) {
        roots.push((
            mount.join(".Trash").join(uid.to_string()),
            Some(mount.to_path_buf()),
        ));
        roots.push((
            mount.join(format!(".Trash-{uid}")),
            Some(mount.to_path_buf()),
        ));
    }
    let mut newest: Option<(String, PathBuf)> = None;
    let mut tied = false;
    for (root, topdir) in roots {
        for index in 1.. {
            let entry = numbered(name, index);
            let mut info_name = entry.clone();
            info_name.push(".trashinfo");
            let Ok(info) = std::fs::read_to_string(root.join("info").join(info_name)) else {
                break;
            };
            let field = |key: &str| info.lines().find_map(|line| line.strip_prefix(key));
            let recorded = field("Path=").and_then(decode_trashinfo_path);
            let recorded = match (recorded, &topdir) {
                (Some(recorded), _) if recorded.is_absolute() => recorded,
                (Some(recorded), Some(topdir)) => topdir.join(recorded),
                _ => continue,
            };
            if recorded != path {
                continue;
            }
            let date = field("DeletionDate=").unwrap_or_default().to_owned();
            match &newest {
                Some((newest_date, _)) if *newest_date > date => {}
                Some((newest_date, _)) if *newest_date == date => tied = true,
                _ => {
                    newest = Some((date, root.join("files").join(&entry)));
                    tied = false;
                }
            }
        }
    }
    identity_at_path(&newest.filter(|_| !tied)?.1)
}

fn numbered(name: &OsStr, index: usize) -> OsString {
    if index == 1 {
        return name.to_owned();
    }
    let bytes = name.as_bytes();
    let dot = bytes
        .iter()
        .position(|&byte| byte == b'.')
        .unwrap_or(bytes.len());
    let mut numbered = bytes[..dot].to_vec();
    numbered.extend_from_slice(format!(".{index}").as_bytes());
    numbered.extend_from_slice(&bytes[dot..]);
    OsString::from_vec(numbered)
}

fn still_open(at: Entry<'_>, error: glib::Error) -> glib::Error {
    let errno = match error.kind::<gio::IOErrorEnum>() {
        Some(gio::IOErrorEnum::PermissionDenied) => Errno::ACCESS,
        Some(gio::IOErrorEnum::Busy) => Errno::BUSY,
        _ => return error,
    };
    match holders::refused_rename(fs(), at, errno) {
        Some(reason) => glib::Error::new(gio::IOErrorEnum::Failed, &reason),
        None => error,
    }
}

pub(super) fn errno_error(error: Errno) -> glib::Error {
    glib::Error::new(
        gio::io_error_from_errno(error.raw_os_error()),
        &os_error_text(error),
    )
}
