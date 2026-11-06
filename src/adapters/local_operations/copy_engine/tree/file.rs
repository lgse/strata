// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations::copy_engine) struct File {
    pub(super) source: Arc<OwnedFd>,
    pub(super) name: OsString,
    pub(super) target: Arc<OwnedFd>,
    pub(super) parent: EntryStat,
    pub(super) claimed: Option<LocalFileIdentity>,
    pub(super) names: Arc<Names>,
    pub(super) mount: Mount,
    pub(super) path: PathBuf,
    pub(super) scope: usize,
    pub(super) recorded_in: Option<Arc<manifest::Folder>>,
    pub(super) completion: Completion,
}

type LinkKey = (usize, u64, u64);
type Target = (PathBuf, (OwnedFd, Option<LocalFileIdentity>));

impl File {
    pub(super) fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        #[cfg(test)]
        let _activity = crate::adapters::local_operations::tests::CopyActivity::start();
        match tree.copy.with_data(|data| self.copy(tree, data)) {
            Ok(role) => {
                if role != Role::Later {
                    tree.copy.add_file(0);
                }
                queue.complete(self.completion);
            }
            Err(failure) => queue.fail(failure.into_error(&self.name)),
        }
    }

    fn copy(&self, tree: &Tree, data: &mut Data) -> Result<Role, Failure> {
        let copy = &*tree.copy;
        let fs = copy.fs;
        copy.check_cancelled()?;
        if copy.counted {
            copy.progress.start_file(&self.name);
        }
        let at = Entry::new(&*self.source, &self.name);
        let (opened, stat) = self.open_source(tree, at)?;
        let key = (self.scope, stat.identity.device, stat.identity.inode);
        let role = self.role(tree, key, &stat);
        if role == Role::Later {
            return Ok(role);
        }
        let source = match opened {
            Some(opened) => opened,
            None => open_one_name(fs, at, &stat, &copy.cancelled)?,
        };
        let (path, (fd, expected)) = self.target_file(tree)?;
        let name = file_name(&path);
        let target = Entry::new(&*self.target, &name);
        let created = fs.stat_fd(fd.as_fd())?;
        self.take_created(tree, &path, (expected, &created))?;
        let written = self.write((copy, data), &source, (fd, target, &created));
        if !matches!(written, Ok(Some(_))) {
            placement::remove_if_unchanged(
                fs,
                target,
                created.identity,
                copy.written_attributes(created.identity),
                AtFlags::empty(),
            );
        }
        let Some(read) = written? else {
            self.not_written(tree, &path, (key, role));
            return Ok(role);
        };
        copy.wrote(created.identity, &read);
        if role == Role::First {
            tree.links
                .written(key, path, created.identity, read.contents.clone());
        }
        if let Some(folder) = &self.recorded_in {
            folder.file(&self.name, read);
        }
        Ok(role)
    }

    /// An SMB client opens one name of a file at a time, so a later name
    /// of a linked file is never opened there, and the others take turns.
    fn open_source(
        &self,
        tree: &Tree,
        at: Entry<'_>,
    ) -> Result<(Option<Opened>, EntryStat), Failure> {
        let copy = &*tree.copy;
        if tree.opens_one_name_per_file() {
            let stat = copy
                .source_stat(at)
                .map_err(|error| open_failure(&self.name, error))?;
            return Ok((None, stat));
        }
        let opened = open_file(copy.fs, at, &copy.cancelled)?;
        let stat = opened.stat;
        Ok((Some(opened), stat))
    }

    fn role(&self, tree: &Tree, key: LinkKey, stat: &EntryStat) -> Role {
        if stat.nlink <= 1 {
            return Role::Independent;
        }
        let name = links::Name {
            key,
            path: self.path.clone(),
            source: self.name.clone(),
            claimed: self.claimed,
            names: self.names.clone(),
            size: stat.size,
            recorded: self.recorded_in.clone().map(|folder| {
                let openable = !tree.opens_one_name_per_file();
                Pending::new(folder, &self.name, stat, openable)
            }),
        };
        tree.links.role(tree.copy.fs, &self.mount, name)
    }

    fn target_file(&self, tree: &Tree) -> Result<Target, Failure> {
        let copy = &*tree.copy;
        if let Some(claimed) = self.claimed {
            let fd = open_made(copy.fs, Entry::new(&*self.target, &file_name(&self.path)))?;
            return Ok((self.path.clone(), (fd, Some(claimed))));
        }
        make_numbered(
            copy,
            &self.names,
            &self.path,
            false,
            RenameCause::Alias,
            |name| self.create_target(tree, name),
        )
    }

    fn create_target(
        &self,
        tree: &Tree,
        name: &OsStr,
    ) -> Result<(OwnedFd, Option<LocalFileIdentity>), Failure> {
        let fs = tree.copy.fs;
        let target = Entry::new(&*self.target, name);
        match fs.create_exclusive(target, Mode::from_raw_mode(0o600)) {
            Ok(fd) => Ok((fd, None)),
            Err(error) if !taken(fs, target, error) => Err(creation_failure(fs, target, error)),
            Err(_) if self.names.adopts() => {
                let identity = adopt(fs, target, Some(&self.names), fresh_file)?;
                tree.made(&self.path.with_file_name(name), identity);
                Ok((open_made(fs, target)?, Some(identity)))
            }
            Err(_) => Err(Failure::Place(PlaceError::Exists)),
        }
    }

    fn take_created(
        &self,
        tree: &Tree,
        path: &Path,
        (expected, created): (Option<LocalFileIdentity>, &EntryStat),
    ) -> Result<(), Failure> {
        let Some(expected) = expected else {
            tree.made(path, created.identity);
            self.names.made(&file_name(path), created.identity);
            return Ok(());
        };
        let ours = match compare(self.mount.identity, expected, None, created) {
            Proof::Same => true,
            Proof::Different => false,
            Proof::Unproven => created.size == 0,
        };
        if ours {
            Ok(())
        } else {
            Err(Failure::Place(PlaceError::Changed))
        }
    }

    /// Takes `fd`, so a failed file is closed before it is removed: NFS
    /// renames an open file aside instead.
    fn write(
        &self,
        (copy, data): (&Copy, &mut Data),
        source: &Opened,
        (fd, target, created): (OwnedFd, Entry<'_>, &EntryStat),
    ) -> Result<Option<manifest::Read>, Failure> {
        let fs = copy.fs;
        let parent = &self.parent;
        let written = Written {
            fd: fd.as_fd(),
            parent,
            mount: &self.mount,
        };
        let Some((snapshot, read)) = copy.copy_file(data, source, written)? else {
            return Ok(None);
        };
        copy.apply_metadata((fd.as_fd(), created), &snapshot, (parent, &self.mount))?;
        copy.sync_written(fd.as_fd(), &self.mount)?;
        fs.close(fd)?;
        copy.settle_times((target, created.identity), &snapshot, &self.mount)?;
        Ok(Some(read))
    }

    fn not_written(&self, tree: &Tree, path: &Path, (key, role): (LinkKey, Role)) {
        if let Some(created) = &tree.created {
            lock(&created.0).retain(|(made, _)| made != path);
        }
        if let Some(folder) = &self.recorded_in {
            folder.keep(&self.name, KeptCause::Changed);
        }
        if role == Role::First {
            tree.links.torn(key);
        }
    }
}

pub(super) fn open_made(fs: &dyn FsOps, target: Entry<'_>) -> Result<OwnedFd, Failure> {
    fs.open_write(target).map_err(|error| match error {
        // A pipe without a reader took the file's place.
        Errno::NXIO => Failure::Place(PlaceError::Changed),
        error => error.into(),
    })
}

pub(in crate::adapters::local_operations::copy_engine) struct Finish {
    pub(super) name: OsString,
    pub(super) target: Arc<OwnedFd>,
    pub(super) created: EntryStat,
    pub(super) parent: EntryStat,
    pub(super) snapshot: Snapshot,
    pub(super) path: PathBuf,
    pub(super) mount: Mount,
    pub(super) scope: usize,
    pub(super) completion: Completion,
}

impl Finish {
    pub(super) fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        let copy = &tree.copy;
        let applied = self.snapshot.apply_to_folder(
            copy.fs,
            self.target.as_fd(),
            &self.created,
            &self.parent,
            &self.mount,
            &copy.refused_xattrs,
        );
        match applied {
            Ok(mode) => {
                tree.links
                    .retime(self.scope, &self.path, self.created.identity, self.snapshot);
                if let Some(mode) = mode {
                    lock(&tree.restrictive).push((
                        self.scope,
                        Restrictive {
                            path: self.path,
                            identity: self.created.identity,
                            mode,
                        },
                    ));
                }
                drop(self.target);
                queue.complete(self.completion);
            }
            Err(error) => queue.fail(Failure::Os(error).into_error(&self.name)),
        }
    }
}
