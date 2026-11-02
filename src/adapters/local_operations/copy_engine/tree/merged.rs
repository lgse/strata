// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations::copy_engine) struct Merged {
    pub(in crate::adapters::local_operations::copy_engine) source: (Arc<OwnedFd>, OsString),
    pub(in crate::adapters::local_operations::copy_engine) target:
        (Arc<OwnedFd>, OsString, Option<LocalFileIdentity>),
    pub(in crate::adapters::local_operations::copy_engine) path: PathBuf,
    pub(in crate::adapters::local_operations::copy_engine) recorded_in:
        Option<Arc<manifest::Folder>>,
    pub(in crate::adapters::local_operations::copy_engine) placing: Option<Arc<Placing>>,
    pub(in crate::adapters::local_operations::copy_engine) parent: Option<MergedNames>,
}

/// Shared by a folder merged into it unless another filesystem is mounted at either side, as a
/// bind mount of the same one shares its rules.
#[derive(Clone, Copy)]
pub(in crate::adapters::local_operations::copy_engine) struct MergedNames {
    pub(super) devices: (u64, u64),
    pub(super) site: Site,
    pub(super) fixing: bool,
}

type Children = Vec<(OsString, FileType, Planned)>;

impl Merged {
    pub(super) fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        if let Err(failure) = self.merge(tree, queue) {
            queue.fail(failure.into_error(&self.source.1));
        }
    }

    fn merge(&self, tree: &Arc<Tree>, queue: &Queue) -> Result<(), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        let (source_parent, source_name) = &self.source;
        let source = fs
            .open_dir(Entry::new(&**source_parent, source_name))
            .map_err(|error| open_failure(source_name, error))?;
        tree.source_in(fs, source.as_fd());
        let (target, target_stat, mount) = self.open_target(tree)?;
        let (target_parent, target_name, _) = &self.target;
        lock(&tree.copies).push(manifest::Published::new(
            target_parent.clone(),
            target_name.clone(),
            target_stat.identity,
        ));
        if copy.durable {
            tree.reach(&mount, target.as_fd(), &self.path)?;
        }
        let source_stat = fs.stat_fd(source.as_fd())?;
        let names = self.names(tree, &mount, (&source_stat, &target_stat, target.as_fd()));
        let recorded = self
            .recorded_in
            .as_ref()
            .map(|folder| folder.folder(source_name, source_stat.identity));
        // A stage made in the live folder inherits its casefolding.
        let naming = Naming::Decide.decide(fs, mount.family, target.as_fd());
        let folder = copy.names.in_folder(&names.site);
        // As the merge planner, which cannot read a folder's casefolding.
        let children = tree.plan(source.as_fd(), &folder, (names.fixing, Key::Folded))?;
        let requested = children
            .iter()
            .map(|(_, _, planned)| planned.target.clone())
            .collect();
        let live = Live {
            names: Names::new(requested, naming, mount.identity, folder),
            site: names.site,
            naming,
            source: Arc::new(source),
            dir: Arc::new(target),
            stat: target_stat,
            mount,
            path: self.path.clone(),
            name: source_name.clone(),
            recorded,
            placing: self.placing.clone(),
        };
        tree.probe_links_in(&mount, live.dir.clone());
        let new = live.merge_existing((tree, queue), children, names)?;
        live.add(tree, queue, new)
    }

    fn open_target(&self, tree: &Tree) -> Result<(OwnedFd, EntryStat, Mount), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        let (target_parent, target_name, expected) = &self.target;
        let target = fs
            .open_dir(Entry::new(&**target_parent, target_name))
            .map_err(|error| open_failure(target_name, error))?;
        let target_stat = fs.stat_fd(target.as_fd())?;
        let mount = copy.mount(target.as_fd());
        if expected.is_some_and(|expected| !mount.may_be(expected, &target_stat)) {
            return Err(Failure::Changed);
        }
        Ok((target, target_stat, mount))
    }

    fn names(
        &self,
        tree: &Tree,
        mount: &Mount,
        (source_stat, target_stat, target): (&EntryStat, &EntryStat, BorrowedFd<'_>),
    ) -> MergedNames {
        let devices = (source_stat.identity.device, target_stat.identity.device);
        if let Some(parent) = self.parent.filter(|parent| parent.devices == devices) {
            return parent;
        }
        let site = tree.copy.site(mount, target);
        MergedNames {
            devices,
            site,
            fixing: tree.crosses(source_stat, target_stat, &site),
        }
    }
}

impl Live {
    /// Queues the merge of each child folder that meets a folder, and returns the rest.
    fn merge_existing(
        &self,
        (tree, queue): (&Arc<Tree>, &Queue),
        children: Children,
        names: MergedNames,
    ) -> Result<Children, Failure> {
        let mut new = Vec::new();
        for (name, kind, planned) in children {
            tree.copy.check_cancelled()?;
            let existing = match tree
                .copy
                .fs
                .stat(Entry::new(&*self.dir, &planned.target), false)
            {
                Ok(existing) => Some(existing),
                Err(Errno::NOENT) => None,
                Err(error) => return Err(Failure::Os(error).named(&name)),
            };
            match existing {
                Some(existing) if existing.is_directory() && kind == FileType::Directory => {
                    let child = (name, planned, existing.identity);
                    self.merge_folder((tree, queue), child, names);
                }
                // A name taken since the merge was planned gets a number
                // when the child is placed.
                _ => new.push((name, kind, planned)),
            }
        }
        Ok(new)
    }

    fn merge_folder(
        &self,
        (tree, queue): (&Arc<Tree>, &Queue),
        (name, planned, existing): (OsString, Planned, LocalFileIdentity),
        names: MergedNames,
    ) {
        planned_rename(&tree.copy, &self.path, &name, &planned);
        let target_name = &planned.target;
        queue.enqueue(TreeJob::Merge(
            tree.clone(),
            Merged {
                source: (self.source.clone(), name),
                target: (self.dir.clone(), target_name.clone(), Some(existing)),
                path: self.path.join(target_name),
                recorded_in: self.recorded.clone(),
                placing: self.placing.clone(),
                parent: Some(names),
            },
        ));
    }
}

pub(super) struct Live {
    pub(super) source: Arc<OwnedFd>,
    pub(super) dir: Arc<OwnedFd>,
    pub(super) stat: EntryStat,
    pub(super) mount: Mount,
    pub(super) path: PathBuf,
    pub(super) names: Arc<Names>,
    pub(super) site: Site,
    pub(super) naming: Naming,
    pub(super) name: OsString,
    pub(super) recorded: Option<Arc<manifest::Folder>>,
    pub(super) placing: Option<Arc<Placing>>,
}

impl Live {
    pub(super) fn add(
        &self,
        tree: &Arc<Tree>,
        queue: &Queue,
        mut new: Vec<(OsString, FileType, Planned)>,
    ) -> Result<(), Failure> {
        let mut in_place = Vec::new();
        if placement::can_place(tree.copy.fs, self.dir.as_fd(), true) == Some(false) {
            (in_place, new) = new
                .into_iter()
                .partition(|(_, kind, _)| *kind == FileType::Directory);
        }
        if !new.is_empty() {
            self.stage(tree, queue, new)?;
        }
        for (name, _, planned) in in_place {
            self.copy_in_place(tree, &name, &planned.target)
                .map_err(|failure| failure.named(&name))?;
            planned_rename(&tree.copy, &self.path, &name, &planned);
        }
        Ok(())
    }

    pub(super) fn stage(
        &self,
        tree: &Arc<Tree>,
        queue: &Queue,
        children: Vec<(OsString, FileType, Planned)>,
    ) -> Result<(), Failure> {
        let fs = tree.copy.fs;
        tree.copy.check_cancelled()?;
        let staged = StagedSibling::create(self.dir.clone(), StageContent::Directory)?;
        let (name, identity) = (staged.name.clone(), staged.identity);
        lock(&tree.stages).push(staged);
        let dir = fs.open_dir(Entry::new(&*self.dir, &name))?;
        if !self.mount.may_be(identity, &fs.stat_fd(dir.as_fd())?) {
            return Err(Failure::Place(PlaceError::Changed));
        }
        let scope = tree.scopes.fetch_add(1, Ordering::Relaxed);
        let made = Made {
            naming: self.naming,
            dir: Arc::new(dir),
            stat: self.stat,
            path: self.path.clone(),
            names: self.names.clone(),
            site: self.site,
            mount: self.mount,
            scope,
            recorded: self.recorded.clone(),
        };
        let place = Place {
            live: self.dir.clone(),
            stage: made.dir.clone(),
            staged: (name, identity),
            path: self.path.clone(),
            names: self.names.clone(),
            mount: self.mount,
            scope,
            name: self.name.clone(),
            placing: self.placing.clone(),
        };
        let group = Group::new(children.len(), TreeJob::Place(tree.clone(), place));
        let making = Making {
            made: &made,
            tree,
            queue,
            source: &self.source,
            group: &group,
        };
        making.make_all(children)
    }

    pub(super) fn copy_in_place(
        &self,
        tree: &Tree,
        name: &OsStr,
        target_name: &OsStr,
    ) -> Result<(), Failure> {
        let copy = &tree.copy;
        let source = Source {
            parent: self.source.clone(),
            name: name.to_os_string(),
            recorded_in: self.recorded.clone(),
        };
        let expected = copy.fs.stat(source.entry(), false)?.identity;
        if expected.kind != FileType::Directory {
            return Err(Failure::Changed);
        }
        let nested = copy.nested();
        let path = self.path.join(target_name);
        let (path, ownership) = make_numbered(
            copy,
            &self.names,
            &path,
            true,
            RenameCause::Taken,
            |candidate| {
                direct_folder(
                    &nested,
                    &source,
                    expected,
                    &self.dir,
                    candidate,
                    tree.workers,
                )
            },
        )?;
        copy.adopt(&nested, &path);
        if let Some(placing) = &self.placing
            && let Some(name) = path.file_name()
        {
            placing.placed(
                copy.fs,
                (self.dir.as_fd(), name),
                &path,
                &ownership,
                &copy.cancelled,
            );
        }
        if let Some(name) = path.file_name() {
            lock(&tree.copies).push(manifest::Published::proven(
                self.dir.clone(),
                name.to_os_string(),
                ownership,
            ));
        }
        lock(&tree.placed).push(path);
        Ok(())
    }
}
