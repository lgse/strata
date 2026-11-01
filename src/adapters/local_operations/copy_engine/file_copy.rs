// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
pub(super) enum FileTarget<'a> {
    Final(&'a OsStr, Taken),
    Hidden,
}

pub(super) struct Stage {
    pub(super) name: OsString,
    pub(super) created: EntryStat,
}

pub(super) struct FileCopy<'a> {
    pub(super) copy: &'a Copy,
    pub(super) dir: BorrowedFd<'a>,
    pub(super) target: Mount,
    pub(super) data: Data,
    pub(super) read: Option<manifest::Read>,
}

impl<'a> FileCopy<'a> {
    pub(super) fn new(copy: &'a Copy, dir: BorrowedFd<'a>) -> Self {
        Self {
            copy,
            dir,
            target: Mount::new(copy.fs, dir),
            data: Data::default(),
            read: None,
        }
    }

    pub(super) fn at<'b>(&self, name: &'b OsStr) -> Entry<'b>
    where
        'a: 'b,
    {
        Entry {
            dir: self.dir,
            name,
        }
    }

    pub(super) fn open(&self, at: Entry<'_>, seen: &EntryStat) -> Result<Opened, Failure> {
        let fs = self.copy.fs;
        if Mount::new(fs, at.dir).family != Family::Smb {
            return open_file(fs, at, &self.copy.cancelled);
        }
        open_one_name(fs, at, seen, &self.copy.cancelled)
    }

    pub(super) fn copy_from(
        &mut self,
        at: Entry<'_>,
        seen: &EntryStat,
        target: FileTarget<'_>,
        recorded_in: Option<&manifest::Folder>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let fs = self.copy.fs;
        self.copy.check_cancelled()?;
        let source = self.open(at, seen)?;
        let parent = fs.stat_fd(self.dir)?;
        let mut tmpfile = None;
        if stage::tmpfile_allowed(self.target.family)
            && self.target.capabilities().tmpfile != Some(false)
        {
            match fs.open_tmpfile(self.dir, Mode::from_raw_mode(0o600)) {
                Ok(fd) => {
                    self.target.learn(|capabilities| {
                        capabilities.tmpfile.get_or_insert(true);
                    });
                    tmpfile = Some(fd);
                }
                // Before Linux 3.11 the flag reads as O_DIRECTORY.
                Err(error) if is_unsupported(error) || error == Errno::ISDIR => {
                    self.target
                        .learn(|capabilities| capabilities.tmpfile = Some(false));
                }
                Err(error) => return Err(error.into()),
            }
        }
        let copied = match tmpfile {
            Some(fd) => self.through_tmpfile(&source, fd, &parent, target),
            None => self.through_named_stage(&source, &parent, target),
        }?;
        if let (Some(folder), Some(read)) = (recorded_in, self.read.take()) {
            self.copy.wrote(copied.1, &read);
            folder.file(at.name, read);
        }
        Ok(copied)
    }

    pub(super) fn in_place(
        &mut self,
        at: Entry<'_>,
        seen: &EntryStat,
        name: &OsStr,
        recorded_in: Option<&manifest::Folder>,
    ) -> Result<LocalFileIdentity, Failure> {
        let fs = self.copy.fs;
        self.copy.check_cancelled()?;
        let source = self.open(at, seen)?;
        if source.stat.identity != seen.identity {
            return Err(Failure::Changed);
        }
        let parent = fs.stat_fd(self.dir)?;
        let identity = self.create(name, &parent, |file, fd| {
            file.read_into(&source, fd, &parent)
        })?;
        if let (Some(folder), Some(read)) = (recorded_in, self.read.take()) {
            self.copy.wrote(identity, &read);
            folder.file(at.name, read);
        }
        self.copy.add_file(0);
        Ok(identity)
    }

    pub(super) fn read_into(
        &mut self,
        source: &Opened,
        target: BorrowedFd<'_>,
        parent: &EntryStat,
    ) -> Result<Snapshot, Failure> {
        let (snapshot, read) = self
            .copy
            .copy_file(
                &mut self.data,
                source,
                Written {
                    fd: target,
                    parent,
                    mount: &self.target,
                },
            )?
            .ok_or(Failure::Changed)?;
        self.read = Some(read);
        Ok(snapshot)
    }

    pub(super) fn link(
        &self,
        tmpfile: BorrowedFd<'_>,
        target: FileTarget<'_>,
    ) -> Result<OsString, Errno> {
        let fs = self.copy.fs;
        match target {
            FileTarget::Final(name, taken) => {
                first_free(self.copy, self.dir, (name, taken, false), |name| {
                    let to = self.at(name);
                    link_tmpfile(fs, &self.target, tmpfile, to).map_err(|error| {
                        match placement::file_creation_error(error) {
                            PlaceError::Exists => Failure::Place(PlaceError::Exists),
                            _ => creation_failure(fs, to, error),
                        }
                    })
                })
                .map(|(name, ())| name)
                .map_err(|failure| match failure {
                    Failure::Os(error) | Failure::NameRefused(error) => error,
                    _ => Errno::EXIST,
                })
            }
            FileTarget::Hidden => placement::reserve_hidden_name(HiddenName::Copy, |name| {
                link_tmpfile(fs, &self.target, tmpfile, self.at(name)).map_err(|error| {
                    match placement::file_creation_error(error) {
                        PlaceError::Exists => PlaceError::Exists,
                        _ => PlaceError::Io(error),
                    }
                })
            })
            .map(|(name, ())| name)
            .map_err(|error| match error {
                PlaceError::Io(error) => error,
                _ => Errno::EXIST,
            }),
        }
    }

    pub(super) fn through_tmpfile(
        &mut self,
        source: &Opened,
        tmpfile: OwnedFd,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let fs = self.copy.fs;
        let snapshot = self.read_into(source, tmpfile.as_fd(), parent)?;
        let created = fs.stat_fd(tmpfile.as_fd())?;
        self.copy.apply_metadata(
            (tmpfile.as_fd(), &created),
            &snapshot,
            (parent, &self.target),
        )?;
        self.copy.sync_written(tmpfile.as_fd(), &self.target)?;
        self.copy.check_cancelled()?;
        let name = match self.link(tmpfile.as_fd(), target) {
            Ok(name) => name,
            Err(error) => {
                return match placement::file_creation_error(error) {
                    PlaceError::Exists => Err(Failure::Place(PlaceError::Exists)),
                    _ if matches!(error, Errno::NOSPC | Errno::DQUOT) || is_fatal(error) => {
                        Err(error.into())
                    }
                    // A stage would be refused that name too; `ENOENT` may
                    // instead mean that unnamed files cannot be linked.
                    _ if matches!(
                        error,
                        Errno::INVAL | Errno::ILSEQ | Errno::NAMETOOLONG | Errno::BADMSG
                    ) =>
                    {
                        Err(Failure::NameRefused(error))
                    }
                    // The written bytes are copied out on the same device, so
                    // the source is never read twice.
                    _ => self
                        .through_stage_from_tmpfile(tmpfile, &created, &snapshot, parent, target),
                };
            }
        };
        if let Err(error) = fs.close(tmpfile) {
            placement::remove_if_unchanged(
                fs,
                self.at(&name),
                created.identity,
                self.copy.written_attributes(created.identity),
                AtFlags::empty(),
            );
            return Err(error.into());
        }
        self.copy.add_file(0);
        Ok((name, created.identity))
    }

    pub(super) fn through_stage_from_tmpfile(
        &mut self,
        tmpfile: OwnedFd,
        tmpfile_stat: &EntryStat,
        snapshot: &Snapshot,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let (fd, stage) = self.create_stage()?;
        let copied = self
            .copy
            .copy_data(
                &mut self.data,
                (tmpfile.as_fd(), tmpfile_stat, Some(&self.target)),
                (
                    Written {
                        fd: fd.as_fd(),
                        parent,
                        mount: &self.target,
                    },
                    false,
                ),
            )
            .and_then(|_| {
                self.copy.apply_metadata(
                    (fd.as_fd(), &stage.created),
                    snapshot,
                    (parent, &self.target),
                )
            });
        drop(tmpfile);
        match copied {
            Ok(()) => self.finish_stage(fd, stage, snapshot, parent, target),
            Err(failure) => {
                drop(fd);
                Err(self.discard(&stage, failure))
            }
        }
    }

    pub(super) fn through_named_stage(
        &mut self,
        source: &Opened,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let (fd, stage) = self.create_stage()?;
        let copied = self
            .read_into(source, fd.as_fd(), parent)
            .and_then(|snapshot| {
                self.copy.apply_metadata(
                    (fd.as_fd(), &stage.created),
                    &snapshot,
                    (parent, &self.target),
                )?;
                Ok(snapshot)
            });
        match copied {
            Ok(snapshot) => self.finish_stage(fd, stage, &snapshot, parent, target),
            Err(failure) => {
                drop(fd);
                Err(self.discard(&stage, failure))
            }
        }
    }

    pub(super) fn create_stage(&self) -> Result<(OwnedFd, Stage), Failure> {
        let (name, (fd, created)) = stage::create_file(self.copy.fs, self.dir)?;
        Ok((fd, Stage { name, created }))
    }

    pub(super) fn finish_stage(
        &mut self,
        fd: OwnedFd,
        stage: Stage,
        snapshot: &Snapshot,
        parent: &EntryStat,
        target: FileTarget<'_>,
    ) -> Result<(OsString, LocalFileIdentity), Failure> {
        let fs = self.copy.fs;
        let _pinned = Pinned::at(
            fs,
            self.at(&stage.name),
            &stage.created,
            self.target.identity,
        );
        let settled = self
            .copy
            .sync_written(fd.as_fd(), &self.target)
            .and_then(|()| fs.close(fd).map_err(Failure::Os))
            .and_then(|()| {
                self.copy.settle_times(
                    (self.at(&stage.name), stage.created.identity),
                    snapshot,
                    &self.target,
                )
            })
            .and_then(|()| self.copy.check_cancelled());
        if let Err(failure) = settled {
            return Err(self.discard(&stage, failure));
        }
        let FileTarget::Final(name, taken) = target else {
            self.copy.add_file(0);
            return Ok((stage.name, stage.created.identity));
        };
        let (copy, dir) = (self.copy, self.dir);
        let staged = Entry {
            dir,
            name: &stage.name,
        };
        let mut created_from = false;
        let published = first_free(copy, dir, (name, taken, false), |name| {
            if !created_from {
                match stage::publish(fs, dir, &stage.name, stage.created.identity, name) {
                    Err(PlaceError::Unsupported) => created_from = true,
                    published => {
                        return published
                            .map(|_| stage.created.identity)
                            .map_err(Failure::Place);
                    }
                }
            }
            self.create_from(staged, stage.created.identity, Some(snapshot), name, parent)
        });
        let published = match published {
            Ok(published) => published,
            // Either name may hold the copy, so neither is removed.
            Err(Failure::Place(PlaceError::Unconfirmed)) if !created_from => {
                return Err(Failure::Place(PlaceError::Unconfirmed));
            }
            Err(failure) => return Err(self.discard(&stage, failure)),
        };
        if created_from && let Err(error) = self.remove_made(&stage.name, stage.created.identity) {
            return Err(Failure::Error(stage_left(&error)));
        }
        self.copy.add_file(0);
        Ok(published)
    }

    pub(super) fn create_from(
        &mut self,
        from: Entry<'_>,
        expected: LocalFileIdentity,
        snapshot: Option<&Snapshot>,
        name: &OsStr,
        parent: &EntryStat,
    ) -> Result<LocalFileIdentity, Failure> {
        let fs = self.copy.fs;
        let source = match fs.open_source(from, false)? {
            Some((fd, stat)) if stat.identity == expected => Opened {
                fd: SourceFd::new(fd),
                stat,
            },
            _ => return Err(Failure::Changed),
        };
        let read;
        let snapshot = match snapshot {
            Some(snapshot) => snapshot,
            None => {
                read = Snapshot::read(fs, source.fd.as_fd(), &source.stat)?;
                &read
            }
        };
        self.create(name, parent, |file, fd| {
            let from = (source.fd.as_fd(), &source.stat, None);
            file.copy.copy_data(
                &mut file.data,
                from,
                (
                    Written {
                        fd,
                        parent,
                        mount: &file.target,
                    },
                    false,
                ),
            )?;
            Ok(snapshot)
        })
    }

    pub(super) fn create<S: Borrow<Snapshot>>(
        &mut self,
        name: &OsStr,
        parent: &EntryStat,
        write: impl FnOnce(&mut Self, BorrowedFd<'_>) -> Result<S, Failure>,
    ) -> Result<LocalFileIdentity, Failure> {
        let fs = self.copy.fs;
        let target = self.at(name);
        let fd = fs
            .create_exclusive(target, Mode::from_raw_mode(0o600))
            .map_err(|error| match creation_failure(fs, target, error) {
                Failure::Os(error) => Failure::Place(placement::file_creation_error(error)),
                refused => refused,
            })?;
        let created = fs.stat_fd(fd.as_fd())?;
        // Takes `fd`, so a failed file is closed before it is removed: NFS
        // renames an open file aside instead.
        let filled = (|fd: OwnedFd| -> Result<(), Failure> {
            let written = write(self, fd.as_fd())?;
            let snapshot: &Snapshot = written.borrow();
            self.copy
                .apply_metadata((fd.as_fd(), &created), snapshot, (parent, &self.target))?;
            self.copy.sync_written(fd.as_fd(), &self.target)?;
            fs.close(fd)?;
            self.copy
                .settle_times((target, created.identity), snapshot, &self.target)
        })(fd);
        match filled {
            Ok(()) => Ok(created.identity),
            Err(failure) => {
                let removed = self.remove_made(name, created.identity);
                Err(self.copy.after_cleanup(failure, removed))
            }
        }
    }

    pub(super) fn remove_made(
        &self,
        name: &OsStr,
        identity: LocalFileIdentity,
    ) -> Result<(), glib::Error> {
        let fs = self.copy.fs;
        remove_own(
            fs,
            self.at(name),
            |current| current.identity == identity,
            |at| {
                fs.unlink(at, AtFlags::empty())
                    .map_err(leftovers::Refused::Os)
            },
        )
    }

    pub(super) fn discard(&self, stage: &Stage, failure: Failure) -> Failure {
        let removed = self.remove_made(&stage.name, stage.created.identity);
        self.copy.after_cleanup(failure, removed)
    }
}
