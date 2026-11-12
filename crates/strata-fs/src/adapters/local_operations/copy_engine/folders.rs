// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn stage_folder(
    copy: &Arc<Copy>,
    source: &Source,
    expected: LocalFileIdentity,
    parent: &Arc<OwnedFd>,
    workers: usize,
) -> Result<StagedSibling, Failure> {
    copy.check_cancelled()?;
    let mut staged = StagedSibling::create(parent.clone(), StageContent::Directory)?;
    let root = Root {
        parent: parent.clone(),
        name: staged.name.clone(),
        identity: staged.identity,
    };
    let created = Arc::new(Created::default());
    match fill(copy, source, expected, &root, workers, Some(&created)) {
        Ok(restrictive) => {
            staged.ownership =
                Some(copy.created_ownership(staged.identity, lock(&created.0).clone()));
            staged.restrictive = restrictive;
            Ok(staged)
        }
        Err(failure) => Err(copy.discarding(staged, failure)),
    }
}

pub(super) fn publish_folder(
    copy: &Arc<Copy>,
    staged: StagedSibling,
    name: &OsStr,
    taken: Taken,
    workers: usize,
) -> Result<(OsString, Ownership), Failure> {
    let ownership = staged
        .ownership
        .clone()
        .unwrap_or_else(|| Ownership::new(staged.identity));
    if let Err(failure) = copy.check_cancelled() {
        return Err(copy.discarding(staged, failure));
    }
    let source = Source {
        parent: staged.parent.clone(),
        name: staged.name.clone(),
        recorded_in: None,
    };
    let mut copied_from = None;
    let published = first_free(copy, staged.parent.as_fd(), (name, taken, true), |name| {
        if copied_from.is_none() {
            match stage::publish(
                copy.fs,
                staged.parent.as_fd(),
                &staged.name,
                staged.identity,
                name,
            ) {
                Err(PlaceError::Unsupported) => {}
                published => {
                    return published.map(|_| ownership.clone()).map_err(Failure::Place);
                }
            }
        }
        let from = copied_from.get_or_insert_with(|| copy.uncounted());
        direct_folder(
            from,
            &source,
            staged.identity,
            &staged.parent,
            name,
            workers,
        )
    });
    match published {
        Ok(published) if copied_from.is_some() => match copy.discard(staged) {
            Ok(()) => Ok(published),
            Err(error) => Err(Failure::Error(stage_left(&error))),
        },
        Ok(published) => Ok(published),
        // Either name may hold the copy, so neither is removed.
        Err(Failure::Place(PlaceError::Unconfirmed)) if copied_from.is_none() => {
            Err(Failure::Place(PlaceError::Unconfirmed))
        }
        Err(failure) => Err(copy.discarding(staged, failure)),
    }
}

pub(super) fn stage_left(error: &glib::Error) -> glib::Error {
    io_error(rust_i18n::t!(
        "The item was copied, but its staging copy could not be removed: %{error}",
        error = crate::services::gio_error_detail(error)
    ))
}

pub(super) struct Root {
    pub(super) parent: Arc<OwnedFd>,
    pub(super) name: OsString,
    pub(super) identity: LocalFileIdentity,
}

pub(super) fn fill(
    copy: &Arc<Copy>,
    source: &Source,
    expected: LocalFileIdentity,
    root: &Root,
    workers: usize,
    created: Option<&Arc<Created>>,
) -> Result<Vec<Restrictive>, Failure> {
    let fs = copy.fs;
    let parent = fs.stat_fd(root.parent.as_fd())?;
    let mount = copy.mount(root.parent.as_fd());
    let folder = tree::Folder {
        source: (source.parent.clone(), source.name.clone(), Some(expected)),
        target: (root.parent.clone(), root.name.clone(), root.identity),
        parent,
        path: PathBuf::new(),
        naming: tree::Naming::Decide,
        site: copy.site(&mount, root.parent.as_fd()),
        mount,
        scope: 0,
        recorded_in: source.recorded_in.clone(),
        completion: None,
    };
    let (
        walked,
        tree::Walked {
            mut restrictive,
            links,
            ..
        },
    ) = tree::walk(copy, workers, created.cloned(), |tree| {
        tree.probe_links_in(&mount, root.parent.clone());
        tree::TreeJob::Folder(tree, folder)
    });
    walked.map_err(Failure::Error)?;
    if restrictive.is_empty() && links.is_empty() {
        return Ok(restrictive);
    }
    let opened = fs.open_dir(Entry::new(&*root.parent, &root.name))?;
    if !mount.may_be(root.identity, &fs.stat_fd(opened.as_fd())?) {
        return Err(Failure::Place(PlaceError::Changed));
    }
    links.make(
        0,
        copy,
        opened.as_fd(),
        Path::new(""),
        &mount,
        created.map(|created| &**created),
    )?;
    if let Err(error) = restrictive::tighten(fs, opened.as_fd(), &mut restrictive, mount.identity) {
        restrictive::loosen(fs, opened.as_fd(), &restrictive, &mount);
        return Err(error.into());
    }
    Ok(restrictive)
}

#[derive(Default)]
pub(super) struct Created(pub(super) Mutex<Vec<(PathBuf, LocalFileIdentity)>>);

impl Created {
    pub(super) fn add(&self, path: PathBuf, identity: LocalFileIdentity) {
        lock(&self.0).push((path, identity));
    }

    pub(super) fn remove(
        &self,
        copy: &Copy,
        root: &Root,
        mount: &Mount,
    ) -> Result<(), glib::Error> {
        let fs = copy.fs;
        let root_entry = Entry::new(&*root.parent, &root.name);
        let dir = match fs.open_dir(root_entry) {
            Ok(dir) => dir,
            Err(Errno::NOENT) => return Ok(()),
            Err(error) => return Err(io_error(os_error_detail(error))),
        };
        let opened = fs
            .stat_fd(dir.as_fd())
            .map_err(|error| io_error(os_error_detail(error)))?;
        if !mount.may_be(root.identity, &opened) {
            return Err(placement_error(PlaceError::Changed));
        }
        let mut made = std::mem::take(&mut *lock(&self.0));
        made.sort_by(|(left, _), (right, _)| {
            let depth = |path: &Path| path.components().count();
            depth(right)
                .cmp(&depth(left))
                .then_with(|| left.parent().cmp(&right.parent()))
        });
        let mut folder: Option<(&Path, OwnedFd)> = None;
        for (path, identity) in &made {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                continue;
            };
            if folder.as_ref().is_none_or(|(opened, _)| *opened != parent) {
                folder = fs
                    .open_dir(Entry {
                        dir: dir.as_fd(),
                        name: restrictive::beneath(parent),
                    })
                    .ok()
                    .map(|fd| (parent, fd));
            }
            let Some((_, fd)) = &folder else {
                continue;
            };
            let entry = Entry::new(fd, name);
            if identity.kind != FileType::Directory {
                let attributes = copy.written_attributes(*identity);
                placement::remove_if_unchanged(fs, entry, *identity, attributes, AtFlags::empty());
                continue;
            }
            let unchanged = || {
                fs.stat(entry, true)
                    .is_ok_and(|current| current.identity == *identity)
            };
            if unchanged() {
                let _ = copy.remove_folder(entry, &unchanged);
            }
        }
        drop((folder, dir));
        let made = |current: &EntryStat| mount.may_be(root.identity, current);
        let unchanged = || {
            fs.stat(root_entry, true)
                .is_ok_and(|current| made(&current))
        };
        remove_own(fs, root_entry, made, |at| {
            copy.remove_folder(at, &unchanged)
        })
    }
}

pub(super) fn direct_folder(
    copy: &Arc<Copy>,
    source: &Source,
    expected: LocalFileIdentity,
    parent: &Arc<OwnedFd>,
    name: &OsStr,
    workers: usize,
) -> Result<Ownership, Failure> {
    let fs = copy.fs;
    copy.check_cancelled()?;
    let target = Entry::new(&**parent, name);
    if let Err(error) = fs.mkdir(target, Mode::from_raw_mode(0o700)) {
        return Err(if tree::taken(fs, target, error) {
            Failure::Place(PlaceError::Exists)
        } else {
            creation_failure(fs, target, error)
        });
    }
    let identity = fs
        .open_dir(target)
        .and_then(|fd| fs.stat_fd(fd.as_fd()))?
        .identity;
    let root = Root {
        parent: parent.clone(),
        name: name.to_os_string(),
        identity,
    };
    let created = Arc::new(Created::default());
    fill(copy, source, expected, &root, workers, Some(&created))
        .map(|_| copy.created_ownership(identity, lock(&created.0).clone()))
        .map_err(|failure| {
            let mount = Mount::new(fs, parent.as_fd());
            let removed = created.remove(copy, &root, &mount);
            copy.after_cleanup(failure, removed)
        })
}

pub(super) fn from_stage(
    copy: &Arc<Copy>,
    source: &Source,
    identity: LocalFileIdentity,
    parent: &Arc<OwnedFd>,
    name: &OsStr,
) -> Result<Ownership, Failure> {
    match identity.kind {
        FileType::Directory => {
            let workers = worker_count(source, parent);
            direct_folder(copy, source, identity, parent, name, workers)
        }
        FileType::RegularFile => {
            let parent_stat = copy.fs.stat_fd(parent.as_fd())?;
            FileCopy::new(copy, parent.as_fd())
                .create_from(source.entry(), identity, None, name, &parent_stat)
                .map(|identity| copy.ownership(identity))
        }
        // The ladder recreates symlinks itself.
        _ => Err(Failure::Place(PlaceError::Unsupported)),
    }
}

pub(super) fn merge(
    copy: &Arc<Copy>,
    source: &Source,
    folder: &Path,
    workers: Option<usize>,
    placing: Option<Arc<Placing>>,
) -> Result<(Vec<Barrier>, Option<Published>), Failure> {
    let (parent, name) = split(folder).map_err(Failure::Error)?;
    let workers = workers.unwrap_or_else(|| worker_count(source, &parent));
    let merged = tree::Merged {
        source: (source.parent.clone(), source.name.clone()),
        target: (parent.clone(), name.clone(), None),
        path: PathBuf::new(),
        recorded_in: source.recorded_in.clone(),
        placing,
        parent: None,
    };
    let (
        walked,
        tree::Walked {
            stages,
            placed,
            mut nested,
            copies,
            ..
        },
    ) = tree::walk(copy, workers, None, |tree| {
        tree::TreeJob::Merge(tree, merged)
    });
    let Err(error) = walked else {
        for barrier in &mut nested {
            barrier.path = folder.join(&barrier.path);
        }
        lock(&copy.copies).extend(copies);
        return Ok((nested, None));
    };
    copy.keep_left_beneath(&placed);
    let mut cleanup = Ok(());
    for staged in stages {
        cleanup = cleanup.and(copy.discard(staged));
    }
    Err(copy.after_cleanup(Failure::Error(error), cleanup))
}
