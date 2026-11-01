// SPDX-License-Identifier: MIT

//! Hard links within one copied item, or one merge stage. Only the first
//! name of a source file with several names is copied. Once every file of
//! its scope is written, each later name is linked to that copy, or copied
//! from it where the destination refuses.

use std::{
    collections::{HashMap, HashSet, hash_map},
    ffi::OsString,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use rustix::{fs::AtFlags, io::Errno};

use super::{
    Copy, Created, Failure, FileCopy, Mount, is_fatal, lock,
    manifest::{Contents, Pending},
    metadata::Snapshot,
    restrictive,
    tree::{self, Names},
};
use crate::{
    adapters::{
        local_operations::{
            fs_ops::{Entry, FsOps},
            native_fs::LocalFileIdentity,
            placement::{self, PlaceError},
        },
        volume::MountKey,
    },
    services::{KeptCause, RenameCause},
};

/// A source file by device and inode, within one scope.
pub(super) type Key = (usize, u64, u64);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Role {
    Independent,
    First,
    Later,
}

pub(super) struct Name {
    pub(super) key: Key,
    pub(super) path: PathBuf,
    pub(super) source: OsString,
    pub(super) claimed: Option<LocalFileIdentity>,
    pub(super) names: Arc<Names>,
    pub(super) size: u64,
    pub(super) recorded: Option<Pending>,
}

#[derive(Default)]
struct First {
    written: Option<(PathBuf, LocalFileIdentity)>,
    contents: Option<Contents>,
    torn: bool,
}

/// Linking another name changes its parent's modification time.
struct Retimed {
    scope: usize,
    path: PathBuf,
    identity: LocalFileIdentity,
    snapshot: Snapshot,
}

#[derive(Default)]
pub(super) struct Links {
    state: Mutex<State>,
    probes: Mutex<HashMap<MountKey, Probe>>,
}

enum Probe {
    In(Arc<OwnedFd>),
    Found(bool),
}

#[derive(Default)]
struct State {
    firsts: HashMap<Key, First>,
    later: Vec<Name>,
    folders: HashSet<(usize, PathBuf)>,
    retimed: Vec<Retimed>,
}

impl Links {
    /// `dir` is a destination folder the copy does not make, so nothing a probe
    /// leaves ends up in the copy.
    pub(super) fn probe_in(&self, mount: MountKey, dir: Arc<OwnedFd>) {
        lock(&self.probes).entry(mount).or_insert(Probe::In(dir));
    }

    /// A probe rules out links that report another inode, as on sshfs.
    fn kept(&self, fs: &dyn FsOps, mount: &Mount) -> bool {
        if let Some(kept) = mount.capabilities().hard_links {
            return kept;
        }
        let mut probes = lock(&self.probes);
        let kept = match probes.get(&mount.key) {
            Some(Probe::Found(kept)) => return *kept,
            Some(Probe::In(dir)) => placement::links_hold(fs, dir.as_fd()),
            None => false,
        };
        probes.insert(mount.key, Probe::Found(kept));
        kept
    }

    pub(super) fn role(&self, fs: &dyn FsOps, mount: &Mount, name: Name) -> Role {
        if !self.kept(fs, mount) {
            return Role::Independent;
        }
        let mut state = lock(&self.state);
        match state.firsts.entry(name.key) {
            hash_map::Entry::Vacant(vacant) => {
                vacant.insert(First::default());
                Role::First
            }
            hash_map::Entry::Occupied(_) => {
                if let Some(folder) = name.path.parent() {
                    state.folders.insert((name.key.0, folder.to_path_buf()));
                }
                state.later.push(name);
                Role::Later
            }
        }
    }

    pub(super) fn written(
        &self,
        key: Key,
        path: PathBuf,
        identity: LocalFileIdentity,
        contents: Option<Contents>,
    ) {
        if let Some(first) = lock(&self.state).firsts.get_mut(&key) {
            first.written = Some((path, identity));
            first.contents = contents;
        }
    }

    pub(super) fn torn(&self, key: Key) {
        if let Some(first) = lock(&self.state).firsts.get_mut(&key) {
            first.torn = true;
        }
    }

    pub(super) fn retime(
        &self,
        scope: usize,
        path: &Path,
        identity: LocalFileIdentity,
        snapshot: Snapshot,
    ) {
        let mut state = lock(&self.state);
        if state.folders.contains(&(scope, path.to_path_buf())) {
            state.retimed.push(Retimed {
                scope,
                path: path.to_path_buf(),
                identity,
                snapshot,
            });
        }
    }

    pub(super) fn take(&self) -> Self {
        Self {
            state: Mutex::new(std::mem::take(&mut *lock(&self.state))),
            probes: Mutex::default(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        lock(&self.state).later.is_empty()
    }

    /// Runs while every folder is still open to its owner.
    pub(super) fn make(
        &self,
        scope: usize,
        copy: &Copy,
        root: BorrowedFd<'_>,
        base: &Path,
        mount: &Mount,
        created: Option<&Created>,
    ) -> Result<(), Failure> {
        let (mut later, firsts, retimed) = {
            let mut state = lock(&self.state);
            let (later, others) = std::mem::take(&mut state.later)
                .into_iter()
                .partition::<Vec<_>, _>(|name| name.key.0 == scope);
            state.later = others;
            let (retimed, others) = std::mem::take(&mut state.retimed)
                .into_iter()
                .partition::<Vec<_>, _>(|folder| folder.scope == scope);
            state.retimed = others;
            let firsts = later
                .iter()
                .filter_map(|name| {
                    let first = state.firsts.get(&name.key)?;
                    Some((
                        name.key,
                        (first.written.clone(), first.contents.clone(), first.torn),
                    ))
                })
                .collect::<HashMap<_, _>>();
            (later, firsts, retimed)
        };
        later.sort_by(|left, right| left.path.cmp(&right.path));
        let mut linking = true;
        for mut name in later {
            copy.check_cancelled()?;
            let made = match firsts.get(&name.key) {
                Some((Some((path, identity)), _, _)) => {
                    make(copy, (root, base), (path, *identity), &name, &mut linking)
                }
                Some((None, _, true)) => {
                    leave(copy, (root, base), &name)
                        .map_err(|failure| failure.named(&name.source))?;
                    if let Some(recorded) = name.recorded.take() {
                        recorded.keep(KeptCause::Changed);
                    }
                    if let Some(created) = created {
                        lock(&created.0).retain(|(path, _)| path != &name.path);
                    }
                    continue;
                }
                _ => Err(Failure::Changed),
            };
            let (path, made) = made.map_err(|failure| failure.named(&name.source))?;
            if let Some(created) = created {
                created.add(path, made);
            }
            if let Some(recorded) = name.recorded.take() {
                let contents = firsts
                    .get(&name.key)
                    .and_then(|(_, contents, _)| contents.clone());
                recorded.made(contents);
            }
            copy.add_file(name.size);
        }
        for folder in retimed {
            let path = beneath(base, &folder.path)?;
            if let Some(fd) =
                restrictive::reopen(copy.fs, root, path, folder.identity, mount.identity)?
            {
                folder.snapshot.apply_times(copy.fs, fd.as_fd())?;
            }
        }
        Ok(())
    }
}

fn beneath<'a>(base: &Path, path: &'a Path) -> Result<&'a Path, Failure> {
    path.strip_prefix(base).map_err(|_| Failure::Changed)
}

/// Where the destination refuses a link, copies the first copy on the same
/// device instead, and stops linking.
fn make(
    copy: &Copy,
    (root, base): (BorrowedFd<'_>, &Path),
    (first, identity): (&Path, LocalFileIdentity),
    name: &Name,
    linking: &mut bool,
) -> Result<(PathBuf, LocalFileIdentity), Failure> {
    let fs = copy.fs;
    let (Some(first_name), Some(file_name)) = (first.file_name(), name.path.file_name()) else {
        return Err(Failure::Changed);
    };
    let first_dir = parent(fs, root, beneath(base, first)?)?;
    let dir = parent(fs, root, beneath(base, &name.path)?)?;
    let from = Entry::new(&first_dir, first_name);
    if let Some(claimed) = name.claimed
        && !placement::remove_if_unchanged(
            fs,
            Entry::new(&dir, file_name),
            claimed,
            None,
            AtFlags::empty(),
        )
    {
        return Err(Failure::Place(PlaceError::Changed));
    }
    tree::make_numbered(
        copy,
        &name.names,
        &name.path,
        false,
        RenameCause::Alias,
        |file_name| {
            let to = Entry::new(&dir, file_name);
            if *linking {
                match fs.link(from, to) {
                    Ok(()) => {
                        linked(fs, from, to, identity)?;
                        name.names.made(file_name, identity);
                        return Ok(identity);
                    }
                    // Only this copy's own link, from a lost NFS reply, reports
                    // the copy's identity.
                    Err(error) if tree::taken(fs, to, error) => {
                        return tree::adopt(fs, to, Some(&name.names), |stat| {
                            stat.identity == identity
                        });
                    }
                    Err(error)
                        if is_fatal(error) || matches!(error, Errno::NOSPC | Errno::DQUOT) =>
                    {
                        return Err(error.into());
                    }
                    Err(_) => *linking = false,
                }
            }
            let parent = fs.stat_fd(dir.as_fd())?;
            let made = FileCopy::new(copy, dir.as_fd())
                .create_from(from, identity, None, file_name, &parent)?;
            name.names.made(file_name, made);
            Ok(made)
        },
    )
}

fn leave(copy: &Copy, (root, base): (BorrowedFd<'_>, &Path), name: &Name) -> Result<(), Failure> {
    let (Some(claimed), Some(file_name)) = (name.claimed, name.path.file_name()) else {
        return Ok(());
    };
    let dir = parent(copy.fs, root, beneath(base, &name.path)?)?;
    if placement::remove_if_unchanged(
        copy.fs,
        Entry::new(&dir, file_name),
        claimed,
        None,
        AtFlags::empty(),
    ) {
        Ok(())
    } else {
        Err(Failure::Place(PlaceError::Changed))
    }
}

fn parent(fs: &dyn FsOps, root: BorrowedFd<'_>, path: &Path) -> Result<OwnedFd, Errno> {
    let parent = path.parent().unwrap_or(Path::new(""));
    fs.open_dir(Entry {
        dir: root,
        name: restrictive::beneath(parent),
    })
}

/// A new link that reports another item linked whatever replaced the copy
/// at `from`, so that second name of it is removed again. Anything else at
/// `to` was put there after the link, and is left alone.
fn linked(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
    identity: LocalFileIdentity,
) -> Result<(), Failure> {
    let made = fs.stat(to, false)?;
    if made.identity == identity {
        return Ok(());
    }
    if fs
        .stat(from, true)
        .is_ok_and(|current| current.identity == made.identity)
    {
        placement::remove_if_unchanged(
            fs,
            to,
            made.identity,
            made.kept_attributes(),
            AtFlags::empty(),
        );
    }
    Err(Failure::Changed)
}
