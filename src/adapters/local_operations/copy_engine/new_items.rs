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
                return copy.retry_item((dir, (names, is_directory, taken)), refused, attempt);
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
            copy_special(copy, source, (parent, name), (&stat, taken))
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
    (stat, taken): (&EntryStat, Taken),
) -> Result<(OsString, Ownership), Failure> {
    copy.check_cancelled()?;
    let mount = copy.mount(parent.as_fd());
    let link = stat.kind == FileType::Symlink;
    let site = copy.site(&mount, parent.as_fd());
    let target = if link {
        Some(checked_link(copy, source, &site)?)
    } else {
        None
    };
    let special = Special {
        copy,
        source,
        parent,
        stat,
        mount,
        site,
        target,
    };
    let staged = special.stage()?;
    if let Err(failure) = special.prepare(&staged) {
        return Err(copy.discarding(staged, failure));
    }
    let published = first_free(copy, parent.as_fd(), (name, taken, false), |name| {
        special.publish(&staged, name)
    });
    special.settle(staged, published)
}

/// A link's target, where the destination holds links to it.
fn checked_link(copy: &Copy, source: &Source, site: &Site) -> Result<OsString, Failure> {
    if !copy.names.profiles.holds_links(site) {
        return Err(Failure::LinkRefused(Errno::OPNOTSUPP));
    }
    let target = tree::read_link(copy.fs, source.entry())?;
    if !copy.names.profiles.holds_link_to(site, &target) {
        return Err(Failure::LinkRefused(Errno::ILSEQ));
    }
    Ok(target)
}

/// The stage's name is valid everywhere, unlike the target.
fn link_stage_refused(error: Errno) -> bool {
    tree::refuses_node(FileType::Symlink, error) || matches!(error, Errno::INVAL | Errno::ILSEQ)
}

fn refuse_link(copy: &Copy, site: &Site, error: Errno) -> Failure {
    let refused = Failure::LinkRefused(error);
    copy.refused_link(site, &refused);
    refused
}

/// A symlink or FIFO copied through a private stage.
struct Special<'a> {
    copy: &'a Arc<Copy>,
    source: &'a Source,
    parent: &'a Arc<OwnedFd>,
    stat: &'a EntryStat,
    mount: Mount,
    site: Site,
    target: Option<OsString>,
}

type Publication = (Ownership, bool);

impl Special<'_> {
    fn is_link(&self) -> bool {
        self.stat.kind == FileType::Symlink
    }

    fn stage(&self) -> Result<StagedSibling, Failure> {
        let content = match &self.target {
            Some(target) => StageContent::Symlink(target.clone()),
            None => StageContent::Fifo,
        };
        // These creation syscalls return no descriptor; retain ownership in a private stage.
        StagedSibling::create(self.parent.clone(), content).map_err(|error| self.not_staged(error))
    }

    fn not_staged(&self, error: PlaceError) -> Failure {
        let PlaceError::Io(errno) = error else {
            return error.into();
        };
        if self.stat.kind == FileType::Fifo && tree::refuses_node(FileType::Fifo, errno) {
            return Failure::NotCopyable;
        }
        if self.is_link() && link_stage_refused(errno) {
            return refuse_link(self.copy, &self.site, errno);
        }
        error.into()
    }

    fn prepare(&self, staged: &StagedSibling) -> Result<(), Failure> {
        let copy = self.copy;
        let at = Entry::new(&**self.parent, &staged.name);
        if self.stat.kind == FileType::Fifo {
            let parent_stat = copy.fs.stat_fd(self.parent.as_fd())?;
            tree::finish_pipe(
                copy,
                self.stat,
                (at, staged.identity),
                (&parent_stat, &self.mount),
            )?;
        } else {
            self.prepare_link(at, staged)?;
        }
        copy.check_cancelled()
    }

    fn prepare_link(&self, at: Entry<'_>, staged: &StagedSibling) -> Result<(), Failure> {
        let copy = self.copy;
        if self.mount.family != Family::Fuse {
            best_effort(copy.fs.set_link_times(at, self.stat.atime, self.stat.mtime))?;
        }
        if copy.recording {
            copy.record_state(staged.identity, &copy.fs.stat(at, false)?);
        }
        Ok(())
    }

    fn publish(&self, staged: &StagedSibling, name: &OsStr) -> Result<Publication, Failure> {
        let copy = self.copy;
        let published = stage::publish(
            copy.fs,
            self.parent.as_fd(),
            &staged.name,
            staged.identity,
            name,
        );
        match published {
            Ok(placement::Placed::RecreatedSymlink) => Ok((
                tree::made_again(
                    copy.fs,
                    Entry::new(&**self.parent, name),
                    self.target.as_deref(),
                ),
                false,
            )),
            Ok(_) => Ok((copy.ownership(staged.identity), false)),
            Err(PlaceError::Unsupported) => {
                self.copy_out(staged, name)?;
                Ok((Ownership::unproven(), true))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn copy_out(&self, staged: &StagedSibling, name: &OsStr) -> Result<(), Failure> {
        let copy = self.copy;
        let from = Entry::new(&**self.parent, &staged.name);
        let to = Entry::new(&**self.parent, name);
        let uncounted = copy.uncounted();
        if self.is_link() {
            tree::copy_link(&uncounted, (from, to), &self.mount, None)?;
            return Ok(());
        }
        let parent_stat = copy.fs.stat_fd(self.parent.as_fd())?;
        let made = tree::copy_fifo(&uncounted, (from, to), (&parent_stat, &self.mount), None)?;
        if made.1.is_none() {
            return Err(Failure::NotCopyable);
        }
        Ok(())
    }

    fn settle(
        &self,
        staged: StagedSibling,
        published: Result<(OsString, Publication), Failure>,
    ) -> Result<(OsString, Ownership), Failure> {
        let copy = self.copy;
        let (name, (ownership, copied_out)) = match published {
            Ok(published) => published,
            Err(Failure::Place(PlaceError::Unconfirmed)) => {
                return Err(Failure::Place(PlaceError::Unconfirmed));
            }
            Err(failure) => return Err(copy.discarding(staged, self.link_failure(failure))),
        };
        if copied_out {
            copy.discard(staged)
                .map_err(|error| Failure::Error(stage_left(&error)))?;
        }
        if let Some(folder) = &self.source.recorded_in {
            folder.entry(&self.source.name, self.stat);
        }
        copy.add_file(if self.is_link() { self.stat.size } else { 0 });
        Ok((name, ownership))
    }

    fn link_failure(&self, failure: Failure) -> Failure {
        let refused = self
            .is_link()
            .then(|| self.copy.refused_link(&self.site, &failure))
            .flatten();
        match refused {
            Some(error) => Failure::LinkRefused(error),
            None => failure,
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
        FileType::Symlink => stage_link(copy, source, parent, &stat),
        FileType::Directory => {
            let workers = workers.unwrap_or_else(|| worker_count(source, &parent));
            stage_folder(copy, source, stat.identity, &parent, workers)
        }
        _ => Err(Failure::NotCopyable),
    }
}

fn stage_link(
    copy: &Arc<Copy>,
    source: &Source,
    parent: Arc<OwnedFd>,
    stat: &EntryStat,
) -> Result<StagedSibling, Failure> {
    copy.check_cancelled()?;
    let site = copy.site(&copy.mount(parent.as_fd()), parent.as_fd());
    let link = checked_link(copy, source, &site)?;
    let staged = match StagedSibling::create(parent, StageContent::Symlink(link)) {
        Err(PlaceError::Io(error)) if link_stage_refused(error) => {
            return Err(refuse_link(copy, &site, error));
        }
        staged => staged?,
    };
    if let Err(error) = set_staged_link_times(copy, &staged, stat) {
        return Err(copy.discarding(staged, error.into()));
    }
    copy.add_file(stat.size);
    if let Some(folder) = source.recorded_in.as_deref() {
        folder.entry(&source.name, stat);
    }
    Ok(staged)
}

/// sshfs sets a link's times on what it points to.
fn set_staged_link_times(
    copy: &Copy,
    staged: &StagedSibling,
    stat: &EntryStat,
) -> Result<(), Errno> {
    if Mount::new(copy.fs, staged.parent.as_fd()).family == Family::Fuse {
        return Ok(());
    }
    let link = Entry::new(&*staged.parent, &staged.name);
    best_effort(copy.fs.set_link_times(link, stat.atime, stat.mtime))
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
