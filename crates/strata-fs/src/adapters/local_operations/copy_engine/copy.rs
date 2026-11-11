// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct Copy {
    pub(super) fs: &'static dyn FsOps,
    pub(super) item: OsString,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) progress: Arc<SharedProgress>,
    /// False for a copy from a stage, whose bytes were counted already and
    /// which is removed afterwards, so the copy may leave nothing out.
    pub(super) counted: bool,
    pub(super) names: NamePolicy,
    pub(super) data: Mutex<Vec<Data>>,
    pub(super) refused_xattrs: Mutex<HashSet<OsString>>,
    pub(super) nodes_refused: AtomicBool,
    pub(super) recording: bool,
    pub(super) durable: bool,
    pub(super) left: Mutex<Left>,
    pub(super) copies: Mutex<Vec<Published>>,
    pub(super) written: Mutex<HashMap<(u64, u64), (LocalFileIdentity, manifest::State)>>,
    pub(super) leftovers: Deadline,
}

/// Where a file's bytes go: the open target, its folder's stat, and its mount.
#[derive(Clone, Copy)]
pub(super) struct Written<'a> {
    pub(super) fd: BorrowedFd<'a>,
    pub(super) parent: &'a EntryStat,
    pub(super) mount: &'a Mount,
}

type RetriedItem<'a> = (BorrowedFd<'a>, ((&'a OsStr, &'a OsStr), bool, Taken));

/// What one pass of a file copy saw: its digest, where it ended, and how unsettled the source was.
type Pass = (
    Option<ring::digest::Digest>,
    Option<u64>,
    Option<manifest::Unsettled>,
);

/// A rewrite within one tick of a coarse clock keeps the times, not
/// always the size the copy read up to. Virtual files report none.
/// `None` where the source changed while it was read.
fn settled_read(
    stat: &EntryStat,
    read: EntryStat,
    (digest, end, unsettled): Pass,
) -> Option<manifest::Read> {
    let whole = end.is_none_or(|end| end == read.size || (stat.size == 0 && read.size == 0));
    if manifest::changed(stat, &read) || !whole {
        return None;
    }
    Some(manifest::Read {
        stat: read,
        contents: digest
            .zip(unsettled)
            .map(|(digest, unsettled)| manifest::Contents::new(digest, unsettled)),
        copied: digest,
    })
}

/// A mode that gives the owner no read, as a copy of another
/// user's file readable through its group or others may have.
fn reopen_for_times(
    fs: &dyn FsOps,
    entry: Entry<'_>,
) -> Result<Option<(OwnedFd, EntryStat)>, Errno> {
    match fs.open_source(entry, false) {
        Err(Errno::ACCESS) => fs.open_write(entry).and_then(|fd| {
            let stat = fs.stat_fd(fd.as_fd())?;
            Ok(Some((fd, stat)))
        }),
        opened => opened,
    }
}

impl Copy {
    pub(super) fn new(
        fs: &'static dyn FsOps,
        item: OsString,
        (cancelled, progress, counted): (Arc<AtomicBool>, Arc<SharedProgress>, bool),
        (options, leftovers): (Options, Deadline),
    ) -> Arc<Self> {
        Arc::new(Self {
            fs,
            item,
            cancelled,
            progress,
            counted,
            names: options.names,
            data: Mutex::default(),
            refused_xattrs: Mutex::default(),
            nodes_refused: AtomicBool::new(false),
            recording: options.manifest,
            durable: options.durable,
            left: Mutex::default(),
            copies: Mutex::default(),
            written: Mutex::default(),
            leftovers,
        })
    }

    pub(super) fn ownership(&self, identity: LocalFileIdentity) -> Ownership {
        self.created_ownership(identity, std::iter::empty())
    }

    pub(super) fn created_ownership(
        &self,
        identity: LocalFileIdentity,
        descendants: impl IntoIterator<Item = (PathBuf, LocalFileIdentity)>,
    ) -> Ownership {
        let descendants = descendants.into_iter().collect::<Vec<_>>();
        let written = lock(&self.written);
        let states = std::iter::once((PathBuf::new(), identity))
            .chain(descendants.iter().cloned())
            .filter_map(|(path, identity)| {
                let (known, state) = written.get(&(identity.device, identity.inode))?;
                (*known == identity).then_some((path, *state))
            })
            .collect::<Vec<_>>();
        Ownership::created(identity, descendants).with_states(states)
    }

    pub(super) fn uncounted(&self) -> Arc<Self> {
        let options = Options {
            names: self.names.clone(),
            durable: self.durable,
            ..Options::default()
        };
        Self::new(
            self.fs,
            self.item.clone(),
            (self.cancelled.clone(), self.progress.clone(), false),
            (options, self.leftovers.clone()),
        )
    }

    pub(super) fn remove_folder(
        &self,
        at: Entry<'_>,
        unchanged: &dyn Fn() -> bool,
    ) -> Result<(), leftovers::Refused> {
        leftovers::remove_folder(self.fs, at, &self.leftovers, &|| false, unchanged)
    }

    /// Forced where the mount caches attributes: a move records the source from this stat.
    pub(super) fn source_stat(&self, at: Entry<'_>) -> Result<EntryStat, Errno> {
        let forced = self.recording && Mount::new(self.fs, at.dir).caches_attributes();
        self.fs.stat(at, forced)
    }

    pub(super) fn mount(&self, dir: BorrowedFd<'_>) -> Mount {
        let (key, capabilities) = self.names.profiles.mount(self.fs, dir);
        Mount::with(key, capabilities)
    }

    pub(super) fn name_limit(&self, dir: BorrowedFd<'_>) -> NameLimit {
        self.folder_rules(dir).limit
    }

    pub(super) fn folder_rules(&self, dir: BorrowedFd<'_>) -> FolderRules {
        self.names.in_folder(&self.site(&self.mount(dir), dir))
    }

    pub(super) fn site(&self, mount: &Mount, dir: BorrowedFd<'_>) -> Site {
        self.names.profiles.site(mount.key, mount.names, dir)
    }

    /// Names given back, as undo does, stay refused. A name planned otherwise than for Keep Both
    /// is reported by the planner.
    pub(super) fn retry_item<T>(
        &self,
        (dir, ((name, refused_name), is_directory, taken)): RetriedItem<'_>,
        refused: Failure,
        mut attempt: impl FnMut(&OsStr) -> Result<T, Failure>,
    ) -> Result<(OsString, T), Failure> {
        if self.names.mode != NameMode::Fix {
            return Err(refused);
        }
        let rules = self.folder_rules(dir);
        let target = (refused_name, is_directory);
        let (made, value, cause) = retry_refused(self, &rules, target, refused, |stricter| {
            let numbered = numbered_names(stricter, is_directory, rules.limit);
            for candidate in std::iter::once(stricter.to_os_string()).chain(numbered) {
                match attempt(&candidate) {
                    Err(Failure::Place(PlaceError::Exists)) => {}
                    made => return made.map(|value| (candidate, value)),
                }
            }
            Err(Failure::Place(PlaceError::Exists))
        })?;
        if taken == Taken::Fail || *name == *self.item {
            lock(&self.left).item = Some((name.to_os_string(), cause));
        }
        Ok((made, value))
    }

    /// A target the mount refuses fails as a name stricter rules could not help. A mount without
    /// links at all learns.
    pub(super) fn refused_link(&self, site: &Site, failure: &Failure) -> Option<Errno> {
        let error = match failure {
            Failure::LinkRefused(error)
            | Failure::NameRefused(error @ (Errno::INVAL | Errno::ILSEQ))
            | Failure::Place(PlaceError::Io(error @ Errno::ILSEQ)) => *error,
            Failure::Place(PlaceError::InvalidName) => Errno::INVAL,
            _ => return None,
        };
        if holds_no_links(error) {
            self.names.profiles.refuse_links(site);
        }
        Some(error)
    }

    pub(super) fn nested(&self) -> Arc<Self> {
        let options = Options {
            names: self.names.clone(),
            workers: None,
            manifest: self.recording,
            durable: self.durable,
        };
        Self::new(
            self.fs,
            self.item.clone(),
            (self.cancelled.clone(), self.progress.clone(), self.counted),
            (options, self.leftovers.clone()),
        )
    }

    pub(super) fn adopt(&self, nested: &Self, path: &Path) {
        let Left {
            renamed, skipped, ..
        } = std::mem::take(&mut *lock(&nested.left));
        let mut left = lock(&self.left);
        left.renamed
            .extend(renamed.into_iter().map(|(requested, saved_as, cause)| {
                (path.join(requested), path.join(saved_as), cause)
            }));
        left.skipped
            .extend(skipped.into_iter().map(|(at, kind)| (path.join(at), kind)));
    }

    pub(super) fn renamed(&self, requested: PathBuf, saved_as: PathBuf, cause: RenameCause) {
        lock(&self.left).renamed.push((requested, saved_as, cause));
    }

    pub(super) fn relocate(&self, from: &Path, to: &Path) {
        let move_beneath = |path: &mut PathBuf| {
            if path != from {
                rebase(path, from, to);
            }
        };
        let mut left = lock(&self.left);
        for (requested, saved_as, _) in &mut left.renamed {
            move_beneath(requested);
            move_beneath(saved_as);
        }
        for (path, _) in &mut left.skipped {
            move_beneath(path);
        }
    }

    pub(super) fn keep_left_beneath(&self, paths: &[PathBuf]) {
        let kept = |path: &Path| paths.iter().any(|placed| path.starts_with(placed));
        let mut left = lock(&self.left);
        left.renamed.retain(|(_, saved_as, _)| kept(saved_as));
        left.skipped.retain(|(path, _)| kept(path));
    }

    pub(super) fn skip(
        &self,
        path: PathBuf,
        (name, recorded_in): (&OsStr, Option<&manifest::Folder>),
        kind: SpecialKind,
    ) -> Result<(), Failure> {
        if !self.counted {
            return Err(Failure::NotCopyable);
        }
        match recorded_in {
            Some(folder) => folder.keep(name, KeptCause::Skipped(kind)),
            None => lock(&self.left).skipped.push((path, kind)),
        }
        Ok(())
    }

    pub(super) fn add_file(&self, bytes: u64) {
        if self.counted {
            self.progress.add_bytes(bytes);
            self.progress.add_file();
        }
    }

    pub(super) fn sync_written(&self, fd: BorrowedFd<'_>, mount: &Mount) -> Result<(), Failure> {
        if self.durable && barrier::each_file(mount) {
            self.fs.sync_data(fd)?;
        }
        Ok(())
    }

    pub(super) fn check_cancelled(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Failure::Cancelled);
        }
        Ok(())
    }

    pub(super) fn with_data<T>(&self, work: impl FnOnce(&mut Data) -> T) -> T {
        let mut data = lock(&self.data).pop().unwrap_or_default();
        let result = work(&mut data);
        lock(&self.data).push(data);
        result
    }

    pub(super) fn copy_data(
        &self,
        data: &mut Data,
        (source, source_stat, source_mount): (BorrowedFd<'_>, &EntryStat, Option<&Mount>),
        (target, counted): (Written<'_>, bool),
    ) -> Result<Option<u64>, Failure> {
        let route = data::Route::new(
            self.fs,
            target.mount,
            source_mount,
            source,
            source_stat,
            target.parent,
        );
        let progress = (counted && self.counted).then_some(&*self.progress);
        data.copy(
            self.fs,
            &self.cancelled,
            target.mount,
            &route,
            (source, target.fd),
            source_stat.size,
            |bytes| {
                if let Some(progress) = progress {
                    progress.add_bytes(bytes);
                }
            },
        )
    }

    pub(super) fn copy_digested(
        &self,
        data: &mut Data,
        (source, source_stat): (BorrowedFd<'_>, &EntryStat),
        (target, counted): (BorrowedFd<'_>, bool),
    ) -> Result<(ring::digest::Digest, u64), Failure> {
        let progress = (counted && self.counted).then_some(&*self.progress);
        data.copy_digested(
            self.fs,
            &self.cancelled,
            (source, target),
            source_stat.size,
            |bytes| {
                if let Some(progress) = progress {
                    progress.add_bytes(bytes);
                }
            },
        )
    }

    /// When recording, a source that changed while it was read is read once
    /// more (`None` if it changed again), and one read within a tick of its
    /// mount's coarse times, or of the destination's, is digested for the
    /// move to compare.
    pub(super) fn copy_file(
        &self,
        data: &mut Data,
        source: &Opened,
        target: Written<'_>,
    ) -> Result<Option<(Snapshot, manifest::Read)>, Failure> {
        let mut stat = source.stat;
        let source_fd = source.fd.as_fd();
        // A folder may hold another mount, so each file asks its own.
        let source_mount = self.recording.then(|| Mount::new(self.fs, source_fd));
        for attempt in 0..2 {
            if attempt > 0 {
                self.fs.set_len(target.fd, 0)?;
            }
            let from = (source_fd, &stat, source_mount.as_ref());
            let pass = self.copy_once(data, from, (target, attempt == 0))?;
            let snapshot = Snapshot::read(self.fs, source_fd, &stat)?;
            if !self.recording {
                let read = manifest::Read {
                    stat,
                    contents: None,
                    copied: None,
                };
                return Ok(Some((snapshot, read)));
            }
            let read = self.fs.stat_fd(source_fd)?;
            if let Some(read) = settled_read(&stat, read, pass) {
                return Ok(Some((snapshot, read)));
            }
            stat = read;
        }
        Ok(None)
    }

    fn copy_once(
        &self,
        data: &mut Data,
        (source_fd, stat, source_mount): (BorrowedFd<'_>, &EntryStat, Option<&Mount>),
        (target, counted): (Written<'_>, bool),
    ) -> Result<Pass, Failure> {
        let now = manifest::now();
        let unsettled = source_mount.and_then(|mount| manifest::unsettled(stat, mount, now));
        // The copy takes the source's modification time, which a file put
        // in its place within a tick of the destination's clock can share.
        let copy_unsettled =
            self.recording && manifest::unsettled(stat, target.mount, now).is_some();
        if unsettled.is_some() || copy_unsettled {
            let (digest, end) =
                self.copy_digested(data, (source_fd, stat), (target.fd, counted))?;
            return Ok((Some(digest), Some(end), unsettled));
        }
        let end = self.copy_data(data, (source_fd, stat, source_mount), (target, counted))?;
        Ok((None, end, unsettled))
    }

    pub(super) fn apply_metadata(
        &self,
        (fd, created): (BorrowedFd<'_>, &EntryStat),
        snapshot: &Snapshot,
        (parent, mount): (&EntryStat, &Mount),
    ) -> Result<(), Failure> {
        snapshot.apply(self.fs, fd, created, parent, mount, &self.refused_xattrs)?;
        let stat = self.fs.stat_fd(fd)?;
        self.record_state(created.identity, &stat);
        Ok(())
    }

    pub(super) fn record_state(&self, identity: LocalFileIdentity, stat: &EntryStat) {
        lock(&self.written).insert(
            (identity.device, identity.inode),
            (identity, manifest::State::of(stat)),
        );
    }

    pub(super) fn written_attributes(&self, identity: LocalFileIdentity) -> Option<Attributes> {
        let written = lock(&self.written);
        let (known, state) = written.get(&(identity.device, identity.inode))?;
        (*known == identity).then(|| state.attributes())
    }

    pub(super) fn settle_state(&self, identity: LocalFileIdentity, stat: &EntryStat) {
        if let Some((known, state)) =
            lock(&self.written).get_mut(&(identity.device, identity.inode))
            && *known == identity
        {
            state.settle(stat);
        }
    }

    /// A copy digested as it was written is compared by content before its
    /// source is deleted, as within a tick another file can take its place.
    pub(super) fn wrote(&self, identity: LocalFileIdentity, read: &manifest::Read) {
        let Some(copied) = read.copied else {
            return;
        };
        if let Some((known, state)) =
            lock(&self.written).get_mut(&(identity.device, identity.inode))
            && *known == identity
        {
            *state = state.holding(copied);
        }
    }

    /// NFSv4 write delegations set the times at close, and FUSE daemons with
    /// a writeback cache and some SMB servers may too, so a closed file gets
    /// its times again: always on NFSv4, and where a check finds them lost.
    pub(super) fn settle_times(
        &self,
        (entry, identity): (Entry<'_>, LocalFileIdentity),
        snapshot: &Snapshot,
        mount: &Mount,
    ) -> Result<(), Failure> {
        if !self.times_lost((entry, identity), snapshot, mount)? {
            return Ok(());
        }
        let fs = self.fs;
        let reopened = match reopen_for_times(fs, entry) {
            Ok(Some((fd, stat))) if mount.may_be(identity, &stat) => fd,
            // Setting times by name needs only ownership.
            Err(Errno::ACCESS) => {
                return self.settle_times_by_name((entry, identity), snapshot, mount);
            }
            Err(error) if is_fatal(error) => return Err(error.into()),
            _ => return Ok(()),
        };
        snapshot.apply_times(fs, reopened.as_fd())?;
        // The copy is known by the times it is left with.
        self.settle_state(identity, &fs.stat_fd(reopened.as_fd())?);
        match fs.close(reopened) {
            Err(error) if is_fatal(error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    fn times_lost(
        &self,
        (entry, identity): (Entry<'_>, LocalFileIdentity),
        snapshot: &Snapshot,
        mount: &Mount,
    ) -> Result<bool, Failure> {
        match mount.family {
            Family::Nfs4 => Ok(true),
            Family::Fuse | Family::Smb => match self.fs.stat(entry, true) {
                Ok(current) if snapshot.times_match(&current) => {
                    // Times moved by less than the check notices stay, so the
                    // copy is known by them from now on.
                    if mount.may_be(identity, &current) {
                        self.settle_state(identity, &current);
                    }
                    Ok(false)
                }
                Ok(_) => Ok(true),
                Err(error) if is_fatal(error) => Err(error.into()),
                Err(_) => Ok(false),
            },
            _ => Ok(false),
        }
    }

    fn settle_times_by_name(
        &self,
        (entry, identity): (Entry<'_>, LocalFileIdentity),
        snapshot: &Snapshot,
        mount: &Mount,
    ) -> Result<(), Failure> {
        let fs = self.fs;
        match fs.stat(entry, true) {
            Ok(current) if mount.may_be(identity, &current) => {
                snapshot.apply_times_at(fs, entry)?;
                if let Ok(settled) = fs.stat(entry, true)
                    && mount.may_be(identity, &settled)
                {
                    self.settle_state(identity, &settled);
                }
                Ok(())
            }
            Err(error) if is_fatal(error) => Err(error.into()),
            _ => Ok(()),
        }
    }

    pub(super) fn discard(&self, staged: StagedSibling) -> Result<(), glib::Error> {
        unlock_stage(&staged);
        let identity = Some(staged.recorded());
        let roots = vec![PinnedEntry {
            parent: staged.parent,
            name: staged.name,
            identity,
        }];
        let workers = local_delete_worker_count(&roots);
        parallel_delete_local_blocking_with_workers(
            roots,
            Arc::new(AtomicBool::new(false)),
            workers,
            &self.leftovers,
        )
        .map_err(io_error)
    }

    pub(super) fn discarding(&self, staged: StagedSibling, failure: Failure) -> Failure {
        let discarded = self.discard(staged);
        self.after_cleanup(failure, discarded)
    }

    pub(super) fn after_cleanup(
        &self,
        failure: Failure,
        cleanup: Result<(), glib::Error>,
    ) -> Failure {
        match cleanup {
            Ok(()) => failure,
            Err(cleanup) => Failure::Error(copy_failure_after_cleanup(
                failure.into_error(&self.item),
                Err(cleanup),
            )),
        }
    }
}
