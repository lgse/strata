// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations::copy_engine) struct Folder {
    pub(in crate::adapters::local_operations::copy_engine) source:
        (Arc<OwnedFd>, OsString, Option<LocalFileIdentity>),
    pub(in crate::adapters::local_operations::copy_engine) target:
        (Arc<OwnedFd>, OsString, LocalFileIdentity),
    pub(in crate::adapters::local_operations::copy_engine) parent: EntryStat,
    pub(in crate::adapters::local_operations::copy_engine) path: PathBuf,
    pub(in crate::adapters::local_operations::copy_engine) naming: Naming,
    pub(in crate::adapters::local_operations::copy_engine) site: Site,
    pub(in crate::adapters::local_operations::copy_engine) mount: Mount,
    /// Names link only within one scope, whose restrictive folders get their
    /// modes together: the made tree, or one merge stage.
    pub(in crate::adapters::local_operations::copy_engine) scope: usize,
    pub(in crate::adapters::local_operations::copy_engine) recorded_in:
        Option<Arc<manifest::Folder>>,
    pub(in crate::adapters::local_operations::copy_engine) completion: Completion,
}

impl Folder {
    pub(super) fn run(self, tree: &Arc<Tree>, queue: &Queue) {
        let name = self.source.1.clone();
        if let Err(failure) = self.fill(tree, queue) {
            queue.fail(failure.into_error(&name));
        }
    }

    pub(super) fn fill(self, tree: &Arc<Tree>, queue: &Queue) -> Result<(), Failure> {
        let copy = &tree.copy;
        let fs = copy.fs;
        let (source_parent, source_name, expected) = &self.source;
        let source = fs
            .open_dir(Entry::new(&**source_parent, source_name))
            .map_err(|error| open_failure(source_name, error))?;
        let source_stat = fs.stat_fd(source.as_fd())?;
        let (source_mount, _) = tree.source_in(fs, source.as_fd());
        if expected.is_some_and(|expected| !source_mount.may_be(expected, &source_stat)) {
            return Err(Failure::Changed);
        }
        let (target_parent, target_name, created) = &self.target;
        let target = fs.open_dir(Entry::new(&**target_parent, target_name))?;
        let target_stat = fs.stat_fd(target.as_fd())?;
        if !self.mount.may_be(*created, &target_stat) {
            return Err(Failure::Place(PlaceError::Changed));
        }
        let snapshot = Snapshot::read(fs, source.as_fd(), &source_stat)?;
        let naming = self.naming.decide(fs, self.mount.family, target.as_fd());
        let fixing = tree.crosses(&source_stat, &target_stat, &self.site);
        let rules = copy.names.in_folder(&self.site);
        let children = tree.plan(source.as_fd(), &rules, (fixing, naming.key()))?;
        let requested = children
            .iter()
            .map(|(_, _, planned)| planned.target.clone())
            .collect();
        let made = Made {
            naming,
            dir: Arc::new(target),
            stat: target_stat,
            path: self.path,
            names: Names::new(requested, naming, self.mount.identity, rules),
            site: self.site,
            mount: self.mount,
            scope: self.scope,
            recorded: self
                .recorded_in
                .map(|folder| folder.folder(source_name, source_stat.identity)),
        };
        let finish = Finish {
            name: source_name.clone(),
            target: made.dir.clone(),
            created: target_stat,
            parent: self.parent,
            snapshot,
            path: made.path.clone(),
            mount: made.mount,
            scope: self.scope,
            completion: self.completion,
        };
        if children.is_empty() {
            finish.run(tree, queue);
            return Ok(());
        }
        let group = Group::new(children.len(), TreeJob::Finish(tree.clone(), finish));
        let source = Arc::new(source);
        let making = Making {
            made: &made,
            tree,
            queue,
            source: &source,
            group: &group,
        };
        making.make_all(children)
    }
}

pub(super) fn target_children(
    fs: &dyn FsOps,
    source: BorrowedFd<'_>,
    rules: &NameProfile,
    (fixing, key): (bool, Key),
) -> Result<Vec<(OsString, FileType, Planned)>, Failure> {
    let children = children(fs, source)?;
    let entries = children
        .iter()
        .map(|(name, kind)| PlanEntry {
            name,
            is_directory: *kind == FileType::Directory,
            fixing,
            repeatable: false,
        })
        .collect::<Vec<_>>();
    let planned = plan_names(&entries, rules, key)?;
    Ok(children
        .into_iter()
        .zip(planned)
        .map(|((name, kind), planned)| (name, kind, planned))
        .collect())
}

pub(super) struct Made {
    pub(super) dir: Arc<OwnedFd>,
    pub(super) stat: EntryStat,
    pub(super) path: PathBuf,
    pub(super) naming: Naming,
    pub(super) names: Arc<Names>,
    pub(super) site: Site,
    pub(super) mount: Mount,
    pub(super) scope: usize,
    pub(super) recorded: Option<Arc<manifest::Folder>>,
}

pub(super) struct Child<'a> {
    pub(super) name: &'a OsStr,
    pub(super) kind: FileType,
    pub(super) target_name: &'a OsStr,
    pub(super) claim: bool,
}

impl Made {
    /// Reported under its own name, since nothing took the planned one.
    fn skip(&self, copy: &Copy, name: &OsStr, kind: SpecialKind) -> Result<(), Failure> {
        copy.skip(self.path.join(name), (name, self.recorded.as_deref()), kind)
    }

    fn record(&self, name: &OsStr, stat: &EntryStat) {
        if let Some(folder) = &self.recorded {
            folder.entry(name, stat);
        }
    }
}

/// Makes the children of one made folder, or enqueues the jobs that copy them.
pub(super) struct Making<'a> {
    pub(super) made: &'a Made,
    pub(super) tree: &'a Arc<Tree>,
    pub(super) queue: &'a Queue,
    pub(super) source: &'a Arc<OwnedFd>,
    pub(super) group: &'a Arc<Group<TreeJob>>,
}

impl Making<'_> {
    pub(super) fn make_all(
        &self,
        children: Vec<(OsString, FileType, Planned)>,
    ) -> Result<(), Failure> {
        let colliding = if self.made.naming == Naming::Claim {
            let names = children
                .iter()
                .map(|(_, _, planned)| planned.target.as_os_str())
                .collect::<Vec<_>>();
            stage::colliding(&names)
        } else {
            vec![false; children.len()]
        };
        for ((name, kind, planned), colliding) in children.into_iter().zip(colliding) {
            self.tree.copy.check_cancelled()?;
            let child = Child {
                name: &name,
                kind,
                target_name: &planned.target,
                claim: colliding,
            };
            let made = self.make(&child).map_err(|failure| failure.named(&name))?;
            if made {
                planned_rename(&self.tree.copy, &self.made.path, &name, &planned);
            }
        }
        Ok(())
    }

    fn make(&self, child: &Child<'_>) -> Result<bool, Failure> {
        let path = self.made.path.join(child.target_name);
        match child.kind {
            FileType::Directory => self.folder(child.name, &path).map(|()| true),
            FileType::RegularFile => self.file(child, &path).map(|()| true),
            FileType::Symlink => self.link(child.name, &path),
            FileType::Fifo => self.fifo(child.name, &path),
            kind => self.skip(child.name, special_kind(kind)).map(|()| false),
        }
    }

    fn skip(&self, name: &OsStr, kind: SpecialKind) -> Result<(), Failure> {
        self.made.skip(&self.tree.copy, name, kind)?;
        self.queue.complete(Some(self.group.clone()));
        Ok(())
    }

    fn folder(&self, name: &OsStr, path: &Path) -> Result<(), Failure> {
        let (copy, made) = (&self.tree.copy, self.made);
        let (names, dir) = (&*made.names, &*made.dir);
        let fs = copy.fs;
        let (path, created) = make_numbered(copy, names, path, true, RenameCause::Alias, |name| {
            claim_folder(fs, Entry::new(dir, name), names)
        })?;
        self.tree.made(&path, created);
        self.queue.enqueue(TreeJob::Folder(
            self.tree.clone(),
            Folder {
                source: (self.source.clone(), name.to_os_string(), None),
                target: (made.dir.clone(), file_name(&path), created),
                parent: made.stat,
                path,
                naming: made.naming,
                site: made.site,
                mount: made.mount,
                scope: made.scope,
                recorded_in: made.recorded.clone(),
                completion: Some(self.group.clone()),
            },
        ));
        Ok(())
    }

    fn file(&self, child: &Child<'_>, path: &Path) -> Result<(), Failure> {
        let (copy, made) = (&self.tree.copy, self.made);
        let (names, dir) = (&*made.names, &*made.dir);
        let (path, claimed) = if child.claim {
            let (path, identity) =
                make_numbered(copy, names, path, false, RenameCause::Alias, |name| {
                    claim_file(copy, Entry::new(dir, name), names, made.mount.family)
                })?;
            self.tree.made(&path, identity);
            (path, Some(identity))
        } else {
            (path.to_path_buf(), None)
        };
        self.queue.enqueue(TreeJob::File(
            self.tree.clone(),
            File {
                source: self.source.clone(),
                name: child.name.to_os_string(),
                target: made.dir.clone(),
                parent: made.stat,
                claimed,
                names: made.names.clone(),
                mount: made.mount,
                path,
                scope: made.scope,
                recorded_in: made.recorded.clone(),
                completion: Some(self.group.clone()),
            },
        ));
        Ok(())
    }

    /// Whether the link was made rather than left out.
    fn link(&self, name: &OsStr, path: &Path) -> Result<bool, Failure> {
        let (copy, made) = (&self.tree.copy, self.made);
        let (names, dir) = (&*made.names, &*made.dir);
        let source = Entry::new(&**self.source, name);
        let linked = copy.names.profiles.holds_links(&made.site).then(|| {
            make_numbered(copy, names, path, false, RenameCause::Alias, |name| {
                copy_link(
                    copy,
                    (source, Entry::new(dir, name)),
                    &made.mount,
                    Some((names, &made.site)),
                )
            })
        });
        let (path, link) = match linked {
            Some(Ok(linked)) => linked,
            Some(Err(failure)) if copy.refused_link(&made.site, &failure).is_none() => {
                return Err(failure);
            }
            _ => {
                self.skip(name, SpecialKind::Symlink)?;
                return Ok(false);
            }
        };
        if self.tree.created.is_some() || names.adopts() {
            self.note_made(&path)?;
        }
        made.record(name, &link);
        self.queue.complete(Some(self.group.clone()));
        Ok(true)
    }

    fn note_made(&self, path: &Path) -> Result<(), Failure> {
        let copy = &self.tree.copy;
        let name = file_name(path);
        let stat = copy.fs.stat(Entry::new(&*self.made.dir, &name), false)?;
        self.tree.made(path, stat.identity);
        self.made.names.made(&name, stat.identity);
        copy.record_state(stat.identity, &stat);
        Ok(())
    }

    /// Whether the pipe was made rather than left out.
    fn fifo(&self, name: &OsStr, path: &Path) -> Result<bool, Failure> {
        let (copy, made) = (&self.tree.copy, self.made);
        let (names, dir) = (&*made.names, &*made.dir);
        let source = Entry::new(&**self.source, name);
        let (path, (pipe, identity)) =
            make_numbered(copy, names, path, false, RenameCause::Alias, |name| {
                copy_fifo(
                    copy,
                    (source, Entry::new(dir, name)),
                    (&made.stat, &made.mount),
                    Some(names),
                )
            })?;
        let Some(identity) = identity else {
            self.skip(name, SpecialKind::Pipe)?;
            return Ok(false);
        };
        self.tree.made(&path, identity);
        names.made(&file_name(&path), identity);
        made.record(name, &pipe);
        self.queue.complete(Some(self.group.clone()));
        Ok(true)
    }
}

pub(super) fn special_kind(kind: FileType) -> SpecialKind {
    if kind == FileType::Socket {
        SpecialKind::Socket
    } else {
        SpecialKind::Device
    }
}

pub(super) fn file_name(path: &Path) -> OsString {
    path.file_name().unwrap_or_default().to_os_string()
}

pub(super) fn children(
    fs: &dyn FsOps,
    folder: BorrowedFd<'_>,
) -> Result<Vec<(OsString, FileType)>, Failure> {
    let mut children = fs.list(folder)?;
    children.retain(|(name, kind)| {
        *kind == FileType::Directory || !leftovers::is_leftover(fs, folder, name)
    });
    children.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (name, kind) in &mut children {
        if *kind == FileType::Unknown {
            *kind = fs
                .stat(Entry { dir: folder, name }, false)
                .map_err(|error| Failure::Os(error).named(name))?
                .kind;
        }
    }
    Ok(children)
}
