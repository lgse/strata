// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
};

use rustix::{
    fs::{FileType, Mode},
    io::Errno,
};
use unicode_normalization::UnicodeNormalization;

use crate::{
    adapters::{
        local_operations::{
            fs_ops::{Entry, EntryStat, FsOps},
            native_fs::LocalFileIdentity,
            placement::{self, PlaceError, Placed},
        },
        volume::Family,
    },
    model::hidden_names::HiddenName,
};

/// Local POSIX filesystems, which link an unnamed file atomically. The SMB
/// client fakes `O_TMPFILE` with a visible file, and FUSE daemons that
/// implement it are unprobed.
pub(super) fn tmpfile_allowed(family: Family) -> bool {
    matches!(
        family,
        Family::Ext
            | Family::Xfs
            | Family::Btrfs
            | Family::F2fs
            | Family::Tmpfs
            | Family::Bcachefs
            | Family::Zfs
            | Family::Overlay
    )
}

/// Where two different names never collide, unless a folder is casefolded.
pub(super) fn names_never_collide(family: Family) -> bool {
    matches!(
        family,
        Family::Ext | Family::Xfs | Family::Btrfs | Family::F2fs | Family::Tmpfs
    )
}

/// Which of a folder's names a filesystem that compares names loosely may
/// take for another of them. Every name is, where one cannot be folded or
/// is shaped like a short name (`PROGRA~1`), which may alias any long name.
pub(super) fn colliding(names: &[&OsStr]) -> Vec<bool> {
    let keys = names
        .iter()
        .map(|name| fold(name))
        .collect::<Option<Vec<_>>>();
    let Some(keys) = keys.filter(|_| !names.iter().any(|name| short_name(name))) else {
        return vec![true; names.len()];
    };
    let mut counts = HashMap::<&str, usize>::new();
    for key in &keys {
        *counts.entry(key).or_default() += 1;
    }
    keys.iter().map(|key| counts[key.as_str()] > 1).collect()
}

/// `BASE~N` of at most eight characters, then at most three of extension:
/// the only shape FAT and Windows generate for a long name, in any case.
fn short_name(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    let (base, extension) = match name.iter().position(|&byte| byte == b'.') {
        Some(dot) => (&name[..dot], &name[dot + 1..]),
        None => (name, &name[name.len()..]),
    };
    let numbered = base
        .iter()
        .rposition(|&byte| byte == b'~')
        .is_some_and(|tilde| {
            tilde > 0 && tilde + 1 < base.len() && base[tilde + 1..].iter().all(u8::is_ascii_digit)
        });
    numbered
        && name.is_ascii()
        && !name.contains(&b' ')
        && base.len() <= 8
        && extension.len() <= 3
        && !extension.contains(&b'.')
}

pub(in crate::adapters::local_operations) fn collision_key(name: &OsStr) -> OsString {
    fold(name).map_or_else(|| name.to_os_string(), OsString::from)
}

/// The same for any two names a filesystem may take for one: compatibility
/// forms decomposed, case folded, and without the trailing dots and spaces
/// Windows drops. It may join names that differ, never split ones that alias.
fn fold(name: &OsStr) -> Option<String> {
    let name = name.to_str()?.trim_end_matches(['.', ' ']);
    if name.is_ascii() {
        return Some(name.to_ascii_lowercase());
    }
    Some(
        name.nfkd()
            .flat_map(char::to_uppercase)
            .flat_map(char::to_lowercase)
            .nfkd()
            .collect(),
    )
}

pub(in crate::adapters::local_operations) fn create_file(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
) -> Result<(OsString, (OwnedFd, EntryStat)), PlaceError> {
    placement::reserve_hidden_name(HiddenName::Copy, |name| {
        let entry = Entry { dir, name };
        let fd = match fs.create_exclusive(entry, Mode::from_raw_mode(0o600)) {
            Ok(fd) => fd,
            Err(Errno::EXIST) => lost_file(fs, entry).ok_or(PlaceError::Exists)?,
            Err(error) => return Err(placement::file_creation_error(error)),
        };
        let stat = fs.stat_fd(fd.as_fd()).map_err(PlaceError::Io)?;
        Ok((fd, stat))
    })
}

fn lost_file(fs: &dyn FsOps, entry: Entry<'_>) -> Option<OwnedFd> {
    let fresh = |stat: &EntryStat| stat.kind == FileType::RegularFile && stat.size == 0;
    let identity = placement::left_by_lost_reply(fs, entry, |stat| fresh(stat) && stat.nlink == 1)?;
    let fd = fs.open_write(entry).ok()?;
    fs.stat_fd(fd.as_fd())
        .is_ok_and(|opened| opened.identity == identity && fresh(&opened))
        .then_some(fd)
}

pub(in crate::adapters::local_operations) fn publish(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    stage: &OsStr,
    recorded: LocalFileIdentity,
    name: &OsStr,
) -> Result<Placed, PlaceError> {
    placement::publish_staged(
        fs,
        Entry { dir, name: stage },
        Entry { dir, name },
        recorded,
    )
}
