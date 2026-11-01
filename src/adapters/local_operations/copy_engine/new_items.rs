// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct Source {
    pub(super) parent: Arc<OwnedFd>,
    pub(super) name: OsString,
    pub(super) recorded_in: Option<Arc<manifest::Folder>>,
}

impl Source {
    pub(super) fn entry(&self) -> Entry<'_> {
        Entry::new(&*self.parent, &self.name)
    }
}

pub(super) fn split(path: &Path) -> Result<(Arc<OwnedFd>, OsString), glib::Error> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io_error(crate::i18n::tr("Invalid local filesystem target")));
    };
    let parent = open_local_parent_directory(parent).map_err(io_error)?;
    Ok((Arc::new(parent), name.to_os_string()))
}

pub(super) fn first_free<T>(
    copy: &Copy,
    dir: BorrowedFd<'_>,
    (name, taken, is_directory): (&OsStr, Taken, bool),
    mut attempt: impl FnMut(&OsStr) -> Result<T, Failure>,
) -> Result<(OsString, T), Failure> {
    for candidate in taken.names(name, is_directory, || copy.name_limit(dir)) {
        match attempt(&candidate) {
            Err(Failure::Place(PlaceError::Exists)) if taken == Taken::Number => {}
            Err(refused) if refused.refuses_name() => {
                let names = (name, candidate.as_os_str());
                return copy.retry_item(dir, (names, is_directory, taken), refused, attempt);
            }
            result => return result.map(|value| (candidate, value)),
        }
    }
    Err(Failure::Place(PlaceError::Exists))
}

/// A mount that learns keeps the rules that made a name. Fails as at first where none is
/// made.
pub(super) fn retry_refused<T>(
    copy: &Copy,
    rules: &FolderRules,
    (name, is_directory): (&OsStr, bool),
    refused: Failure,
    mut make: impl FnMut(&OsStr) -> Result<(OsString, T), Failure>,
) -> Result<(OsString, T, RenameCause), Failure> {
    for (strictness, stricter, cause) in stricter_names(name, &rules.fixed, is_directory) {
        match make(&stricter) {
            Ok((made, value)) => {
                if rules.site.rules.learns {
                    copy.names.profiles.learn(&rules.site, strictness);
                }
                return Ok((made, value, cause));
            }
            Err(failure) if failure.refuses_name() => {}
            Err(failure) => return Err(failure),
        }
    }
    Err(refused)
}

#[derive(Clone, Copy)]
pub(super) enum NewItem {
    Published(Taken),
    InPlace(LocalFileIdentity),
}

pub(super) fn copy_new(
    copy: &Arc<Copy>,
    source: &Source,
    (parent, name): (&Arc<OwnedFd>, &OsStr),
    new: NewItem,
    workers: Option<usize>,
) -> Result<(OsString, Ownership), Failure> {
    let stat = copy.source_stat(source.entry())?;
    let taken = match new {
        NewItem::Published(taken) => taken,
        NewItem::InPlace(expected) if expected == stat.identity => Taken::Fail,
        NewItem::InPlace(_) => return Err(Failure::Changed),
    };
    let in_place = matches!(new, NewItem::InPlace(_));
    let recorded_in = source.recorded_in.as_deref();
    match stat.kind {
        FileType::RegularFile if in_place => FileCopy::new(copy, parent.as_fd())
            .in_place(source.entry(), &stat, name, recorded_in)
            .map(|identity| (name.to_os_string(), copy.ownership(identity))),
        FileType::RegularFile => FileCopy::new(copy, parent.as_fd())
            .copy_from(
                source.entry(),
                &stat,
                FileTarget::Final(name, taken),
                recorded_in,
            )
            .map(|(name, identity)| (name, copy.ownership(identity))),
        FileType::Symlink | FileType::Fifo => {
            copy_special(copy, source, (parent, name), &stat, taken)
        }
        FileType::Directory => {
            let workers = workers.unwrap_or_else(|| worker_count(source, parent));
            if in_place || placement::can_place(copy.fs, parent.as_fd(), true) == Some(false) {
                return first_free(copy, parent.as_fd(), (name, taken, true), |name| {
                    direct_folder(copy, source, stat.identity, parent, name, workers)
                });
            }
            let staged = stage_folder(copy, source, stat.identity, parent, workers)?;
            publish_folder(copy, staged, name, taken, workers)
        }
        _ => Err(Failure::NotCopyable),
    }
}

pub(super) fn copy_special(
    copy: &Arc<Copy>,
    source: &Source,
    (parent, name): (&Arc<OwnedFd>, &OsStr),
    stat: &EntryStat,
    taken: Taken,
) -> Result<(OsString, Ownership), Failure> {
    copy.check_cancelled()?;
    let mount = copy.mount(parent.as_fd());
    let link = stat.kind == FileType::Symlink;
    let site = copy.site(&mount, parent.as_fd());
    if link && !copy.names.profiles.holds_links(&site) {
        return Err(Failure::LinkRefused(Errno::OPNOTSUPP));
    }
    let target = link
        .then(|| tree::read_link(copy.fs, source.entry()))
        .transpose()?;
    if let Some(target) = &target
        && !copy.names.profiles.holds_link_to(&site, target)
    {
        return Err(Failure::LinkRefused(Errno::ILSEQ));
    }
    let content = match &target {
        Some(target) => StageContent::Symlink(target.clone()),
        None => StageContent::Fifo,
    };
    // These creation syscalls return no descriptor; retain ownership in a private stage.
    let staged = StagedSibling::create(parent.clone(), content).map_err(|error| {
        match error {
            PlaceError::Io(error)
                if stat.kind == FileType::Fifo && tree::refuses_node(FileType::Fifo, error) =>
            {
                Failure::NotCopyable
            }
            // The stage's name is valid everywhere, unlike the target.
            PlaceError::Io(error)
                if link
                    && (tree::refuses_node(FileType::Symlink, error)
                        || matches!(error, Errno::INVAL | Errno::ILSEQ)) =>
            {
                let refused = Failure::LinkRefused(error);
                copy.refused_link(&site, &refused);
                refused
            }
            error => error.into(),
        }
    })?;
    let prepared = (|| {
        let at = Entry::new(&**parent, &staged.name);
        if stat.kind == FileType::Fifo {
            let parent_stat = copy.fs.stat_fd(parent.as_fd())?;
            tree::finish_pipe(copy, stat, (at, staged.identity), &parent_stat, &mount)?;
        } else {
            if mount.family != Family::Fuse {
                best_effort(copy.fs.set_link_times(at, stat.atime, stat.mtime))?;
            }
            if copy.recording {
                copy.record_state(staged.identity, &copy.fs.stat(at, false)?);
            }
        }
        copy.check_cancelled()
    })();
    if let Err(failure) = prepared {
        return Err(copy.discarding(staged, failure));
    }
    let published =
        first_free(
            copy,
            parent.as_fd(),
            (name, taken, false),
            |name| match stage::publish(
                copy.fs,
                parent.as_fd(),
                &staged.name,
                staged.identity,
                name,
            ) {
                Ok(placement::Placed::RecreatedSymlink) => Ok((
                    tree::made_again(copy.fs, Entry::new(&**parent, name), target.as_deref()),
                    false,
                )),
                Ok(_) => Ok((copy.ownership(staged.identity), false)),
                Err(PlaceError::Unsupported) => {
                    let from = Entry::new(&**parent, &staged.name);
                    let to = Entry::new(&**parent, name);
                    let uncounted = copy.uncounted();
                    if stat.kind == FileType::Symlink {
                        tree::copy_link(&uncounted, from, to, &mount, None)?;
                    } else {
                        let parent_stat = copy.fs.stat_fd(parent.as_fd())?;
                        if tree::copy_fifo(&uncounted, from, to, &parent_stat, &mount, None)?
                            .1
                            .is_none()
                        {
                            return Err(Failure::NotCopyable);
                        }
                    }
                    Ok((Ownership::unproven(), true))
                }
                Err(error) => Err(error.into()),
            },
        );
    match published {
        Ok((name, (ownership, copied_out))) => {
            if copied_out {
                copy.discard(staged)
                    .map_err(|error| Failure::Error(stage_left(&error)))?;
            }
            if let Some(folder) = &source.recorded_in {
                folder.entry(&source.name, stat);
            }
            copy.add_file(if stat.kind == FileType::Symlink {
                stat.size
            } else {
                0
            });
            Ok((name, ownership))
        }
        Err(Failure::Place(PlaceError::Unconfirmed)) => {
            Err(Failure::Place(PlaceError::Unconfirmed))
        }
        Err(failure) => {
            let failure = match link.then(|| copy.refused_link(&site, &failure)).flatten() {
                Some(error) => Failure::LinkRefused(error),
                None => failure,
            };
            Err(copy.discarding(staged, failure))
        }
    }
}

pub(super) fn copy_stage(
    copy: &Arc<Copy>,
    source: &Source,
    target: &Path,
    workers: Option<usize>,
) -> Result<StagedSibling, Failure> {
    let (parent, _) = split(target).map_err(Failure::Error)?;
    let stat = copy.source_stat(source.entry())?;
    let recorded_in = source.recorded_in.as_deref();
    match stat.kind {
        FileType::RegularFile => {
            let (name, identity) = FileCopy::new(copy, parent.as_fd()).copy_from(
                source.entry(),
                &stat,
                FileTarget::Hidden,
                recorded_in,
            )?;
            Ok(StagedSibling {
                ownership: Some(copy.ownership(identity)),
                parent,
                name,
                identity,
                restrictive: Vec::new(),
            })
        }
        FileType::Symlink => {
            copy.check_cancelled()?;
            let site = copy.site(&copy.mount(parent.as_fd()), parent.as_fd());
            if !copy.names.profiles.holds_links(&site) {
                return Err(Failure::LinkRefused(Errno::OPNOTSUPP));
            }
            let link = tree::read_link(copy.fs, source.entry())?;
            if !copy.names.profiles.holds_link_to(&site, &link) {
                return Err(Failure::LinkRefused(Errno::ILSEQ));
            }
            let staged = match StagedSibling::create(parent, StageContent::Symlink(link)) {
                // The stage's name is valid everywhere, unlike the target.
                Err(PlaceError::Io(error))
                    if tree::refuses_node(FileType::Symlink, error)
                        || matches!(error, Errno::INVAL | Errno::ILSEQ) =>
                {
                    let refused = Failure::LinkRefused(error);
                    copy.refused_link(&site, &refused);
                    return Err(refused);
                }
                staged => staged?,
            };
            // sshfs sets a link's times on what it points to.
            if Mount::new(copy.fs, staged.parent.as_fd()).family != Family::Fuse {
                let link = Entry::new(&*staged.parent, &staged.name);
                if let Err(error) =
                    best_effort(copy.fs.set_link_times(link, stat.atime, stat.mtime))
                {
                    return Err(copy.discarding(staged, error.into()));
                }
            }
            copy.add_file(stat.size);
            if let Some(folder) = recorded_in {
                folder.entry(&source.name, &stat);
            }
            Ok(staged)
        }
        FileType::Directory => {
            let workers = workers.unwrap_or_else(|| worker_count(source, &parent));
            stage_folder(copy, source, stat.identity, &parent, workers)
        }
        _ => Err(Failure::NotCopyable),
    }
}

pub(super) fn worker_count(source: &Source, target: &OwnedFd) -> usize {
    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let rotational = [source.parent.as_fd(), target.as_fd()]
        .into_iter()
        .any(|fd| fs().rotational(fd) == Some(true));
    bounded_local_worker_count(available, rotational)
}
