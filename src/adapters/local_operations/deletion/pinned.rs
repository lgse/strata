// SPDX-License-Identifier: MIT

use std::{
    ffi::{OsStr, OsString},
    os::fd::{AsFd, OwnedFd},
};

use super::super::{
    fs_ops::{Entry, EntryStat, fs},
    leftovers,
    mount_profile::identity_at,
    native_fs::{self, Recorded, local_directory_children, os_error_detail, retry_local_open},
};
use crate::model::hidden_names;

pub(in crate::adapters::local_operations) enum LocalDeleteStep {
    Removed,
    Leftover,
    Directory {
        handle: OwnedFd,
        children: Vec<OsString>,
    },
}

pub(super) fn ensure_expected_local_identity<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    current: &EntryStat,
    expected: Option<Recorded>,
) -> Result<(), String> {
    let unchanged = |expected: Recorded| {
        native_fs::removable(
            expected,
            current,
            || identity_at(fs(), parent.as_fd()),
            hidden_names::classify(name).is_some(),
        )
    };
    if expected.is_some_and(|expected| !unchanged(expected)) {
        return Err(rust_i18n::t!(
            "%{name} changed while the operation was in progress",
            name = name.to_string_lossy()
        )
        .into_owned());
    }
    Ok(())
}

/// Check the name too: an open descriptor can outlive a rename.
pub(in crate::adapters::local_operations) fn ensure_local_delete_target_unchanged<
    ParentFd: AsFd,
    TargetFd: AsFd,
>(
    parent: &ParentFd,
    name: &OsStr,
    target: &TargetFd,
) -> Result<(), String> {
    let named = fs()
        .stat(Entry::new(parent, name), false)
        .map_err(|error| changed_while_deleting(name, error))?;
    let opened = fs()
        .stat_fd(target.as_fd())
        .map_err(|error| recheck_failed(name, error))?;
    if named.identity != opened.identity {
        return Err(rust_i18n::t!(
            "%{name} changed while it was being deleted",
            name = name.to_string_lossy()
        )
        .into_owned());
    }
    Ok(())
}

fn changed_while_deleting(name: &OsStr, error: rustix::io::Errno) -> String {
    rust_i18n::t!(
        "%{name} changed while it was being deleted: %{error}",
        name = name.to_string_lossy(),
        error = os_error_detail(error)
    )
    .into_owned()
}

fn recheck_failed(name: &OsStr, error: rustix::io::Errno) -> String {
    rust_i18n::t!(
        "Could not recheck %{name}: %{error}",
        name = name.to_string_lossy(),
        error = os_error_detail(error)
    )
    .into_owned()
}

pub(in crate::adapters::local_operations) fn open_local_delete_target<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    expected: Option<Recorded>,
    leftovers_stay: bool,
) -> Result<LocalDeleteStep, String> {
    let is_leftover = || leftovers_stay && leftovers::is_leftover(fs(), parent.as_fd(), name);
    let stat = match fs().stat(Entry::new(parent, name), false) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) if is_leftover() => return Ok(LocalDeleteStep::Leftover),
        Err(error) => return Err(inspect_failed(name, error)),
    };
    ensure_expected_local_identity(parent, name, &stat, expected)?;
    if stat.is_directory() {
        return open_directory_target(parent, name, expected);
    }
    if is_leftover() {
        return Ok(LocalDeleteStep::Leftover);
    }
    unlink_local_file(parent, name)?;
    Ok(LocalDeleteStep::Removed)
}

fn open_directory_target<Fd: AsFd>(
    parent: &Fd,
    name: &OsStr,
    expected: Option<Recorded>,
) -> Result<LocalDeleteStep, String> {
    let handle = open_pinned_directory(parent, name)?;
    let opened = fs()
        .stat_fd(handle.as_fd())
        .map_err(|error| recheck_failed(name, error))?;
    ensure_expected_local_identity(parent, name, &opened, expected)?;
    let children = local_directory_children(&handle)?;
    Ok(LocalDeleteStep::Directory { handle, children })
}

fn inspect_failed(name: &OsStr, error: rustix::io::Errno) -> String {
    rust_i18n::t!(
        "Could not inspect %{name}: %{error}",
        name = name.to_string_lossy(),
        error = os_error_detail(error)
    )
    .into_owned()
}

fn unlink_local_file<Fd: AsFd>(parent: &Fd, name: &OsStr) -> Result<(), String> {
    fs().unlink(Entry::new(parent, name), rustix::fs::AtFlags::empty())
        .map_err(|error| {
            rust_i18n::t!(
                "Could not delete “%{name}”: %{error}",
                name = name.to_string_lossy(),
                error = os_error_detail(error)
            )
            .into_owned()
        })
}

/// Reject symlinks swapped in after the type check, including intermediate components.
fn open_pinned_directory<Fd: AsFd>(parent: &Fd, name: &OsStr) -> Result<OwnedFd, String> {
    retry_local_open(|| {
        fs().open_at(
            Entry::new(parent, name),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            Some(
                rustix::fs::ResolveFlags::BENEATH
                    | rustix::fs::ResolveFlags::NO_SYMLINKS
                    | rustix::fs::ResolveFlags::NO_MAGICLINKS,
            ),
        )
    })
    .map_err(|error| changed_while_deleting(name, error))
}
