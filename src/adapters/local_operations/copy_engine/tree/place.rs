// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations::copy_engine) struct Place {
    pub(super) live: Arc<OwnedFd>,
    pub(super) stage: Arc<OwnedFd>,
    pub(super) staged: (OsString, LocalFileIdentity),
    pub(super) path: PathBuf,
    pub(super) names: Arc<Names>,
    pub(super) mount: Mount,
    pub(super) scope: usize,
    pub(super) name: OsString,
    pub(super) placing: Option<Arc<Placing>>,
}

impl Place {
    pub(super) fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        let name = self.name.clone();
        if let Err(failure) = self.place(tree) {
            queue.fail(failure.into_error(&name));
        }
    }

    fn place(self, tree: &Tree) -> Result<(), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        tree.links.make(
            self.scope,
            copy,
            self.stage.as_fd(),
            &self.path,
            &self.mount,
            None,
        )?;
        let mut children = fs.list(self.stage.as_fd())?;
        children.sort_by(|(left, _), (right, _)| left.cmp(right));
        let mut restrictive = tree.take_restrictive(self.scope);
        let (placed, result) = self.place_children(tree, &children, &mut restrictive);
        self.record_placed(tree, &placed);
        let placed = placed.into_iter().map(|(path, _)| path).collect::<Vec<_>>();
        let tightened = self.tighten(fs, restrictive, &placed);
        lock(&tree.placed).extend(placed);
        result?;
        tightened?;
        self.remove_stage(tree)
    }

    /// Stops at the first child that cannot be placed.
    fn place_children(
        &self,
        tree: &Tree,
        children: &[(OsString, FileType)],
        restrictive: &mut [Restrictive],
    ) -> (Vec<PlacedChild>, Result<(), Failure>) {
        let copy = &tree.copy;
        let mut placed = Vec::new();
        for (name, kind) in children {
            match self.place_child(copy, name, *kind == FileType::Directory) {
                Ok((path, ownership)) => {
                    self.relocated(copy, (name, &path), restrictive);
                    placed.push((path, ownership));
                }
                Err(failure) => {
                    // Either name may hold the child, so the stage is kept.
                    if matches!(failure, Failure::Place(PlaceError::Unconfirmed)) {
                        tree.settle_stage(self.staged.1);
                    }
                    return (placed, Err(failure.named(name)));
                }
            }
        }
        (placed, Ok(()))
    }

    fn relocated(
        &self,
        copy: &Copy,
        (name, path): (&OsStr, &Path),
        restrictive: &mut [Restrictive],
    ) {
        let requested = self.path.join(name);
        if path == requested {
            return;
        }
        copy.relocate(&requested, path);
        for folder in restrictive {
            rebase(&mut folder.path, &requested, path);
        }
    }

    fn record_placed(&self, tree: &Tree, placed: &[PlacedChild]) {
        let copy = &tree.copy;
        let named = || {
            placed
                .iter()
                .filter_map(|(path, ownership)| Some((path.file_name()?, path, ownership)))
        };
        // Placing a name of a file changes the change time of its others.
        if let Some(placing) = &self.placing {
            for (name, path, ownership) in named() {
                placing.placed(
                    copy.fs,
                    (self.live.as_fd(), name),
                    path,
                    ownership,
                    &copy.cancelled,
                );
            }
        }
        for (name, _, ownership) in named() {
            lock(&tree.copies).push(manifest::Published::proven(
                self.live.clone(),
                name.to_os_string(),
                ownership.clone(),
            ));
        }
    }

    fn tighten(
        &self,
        fs: &dyn FsOps,
        restrictive: Vec<Restrictive>,
        placed: &[PathBuf],
    ) -> Result<(), Errno> {
        let mut placed_folders = restrictive
            .into_iter()
            .filter(|folder| placed.iter().any(|path| folder.path.starts_with(path)))
            .filter_map(|folder| {
                let path = folder.path.strip_prefix(&self.path).ok()?.to_path_buf();
                Some(Restrictive { path, ..folder })
            })
            .collect::<Vec<_>>();
        restrictive::tighten(
            fs,
            self.live.as_fd(),
            &mut placed_folders,
            self.mount.identity,
        )
    }

    fn place_child(
        &self,
        copy: &Arc<Copy>,
        name: &OsStr,
        is_directory: bool,
    ) -> Result<PlacedChild, Failure> {
        copy.check_cancelled()?;
        let fs = copy.fs;
        let staged = Entry::new(&*self.stage, name);
        let identity = fs.stat(staged, false)?.identity;
        let ownership = manifest::Ownership::capture(fs, staged, identity)?;
        let link = (identity.kind == FileType::Symlink)
            .then(|| read_link(fs, staged))
            .transpose()?;
        let path = self.path.join(name);
        make_numbered(
            copy,
            &self.names,
            &path,
            is_directory,
            RenameCause::Taken,
            |candidate| {
                let target = Entry::new(&*self.live, candidate);
                match placement::place_staged(fs, staged, target, identity) {
                    Ok(placement::Placed::RecreatedSymlink) => {
                        Ok(made_again(fs, target, link.as_deref()))
                    }
                    Ok(_) => Ok(ownership.clone()),
                    Err(PlaceError::Unsupported) => {
                        self.copy_out(copy, (name, identity), candidate)
                    }
                    Err(error) => Err(error.into()),
                }
            },
        )
    }

    fn copy_out(
        &self,
        copy: &Arc<Copy>,
        (name, identity): (&OsStr, LocalFileIdentity),
        candidate: &OsStr,
    ) -> Result<manifest::Ownership, Failure> {
        let source = Source {
            parent: self.stage.clone(),
            name: name.to_os_string(),
            recorded_in: None,
        };
        let ownership = from_stage(&copy.uncounted(), &source, identity, &self.live, candidate)?;
        copy.discard(StagedSibling {
            ownership: None,
            parent: self.stage.clone(),
            name: name.to_os_string(),
            identity,
            restrictive: Vec::new(),
        })
        .map_err(|error| Failure::Error(stage_left(&error)))?;
        Ok(ownership)
    }

    fn remove_stage(self, tree: &Tree) -> Result<(), Failure> {
        let fs = tree.copy.fs;
        let Self {
            live,
            stage: opened,
            staged: (name, identity),
            ..
        } = self;
        let stage = Entry::new(&*live, &name);
        let held = fs.stat_fd(opened.as_fd()).ok().map(|held| held.identity);
        let unchanged = || {
            fs.stat(stage, false)
                .is_ok_and(|current| held == Some(current.identity))
        };
        let ours = unchanged();
        drop(opened);
        let removed = if ours {
            match tree.copy.remove_folder(stage, &unchanged) {
                Ok(()) | Err(Refused::Os(Errno::NOENT) | Refused::Changed(_)) => Ok(()),
                Err(refused) => Err(refused.errno()),
            }
        } else {
            Ok(())
        };
        tree.settle_stage(identity);
        removed.map_err(|error| Failure::Error(stage_left(&io_error(os_error_detail(error)))))
    }
}

type PlacedChild = (PathBuf, manifest::Ownership);
