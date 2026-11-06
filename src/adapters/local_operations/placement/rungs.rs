// SPDX-License-Identifier: MIT

//! The rungs tried once `renameat2(NOREPLACE)` is refused.

use super::*;

pub(super) fn fallback(
    placing: Placing<'_>,
    expected: Option<LocalFileIdentity>,
    error: Errno,
) -> Result<Placed, PlaceError> {
    let mount = Mount::new(placing.fs, placing.to.dir);
    let source = match after_refused_rename(&placing, &mount, expected, error) {
        ControlFlow::Break(placed) => return placed,
        ControlFlow::Continue(source) => source,
    };
    let source = match source {
        Some(source) => source,
        None => stat_source(placing.fs, placing.from)?,
    };
    if expected.is_some_and(|expected| expected != source.identity) {
        return Err(PlaceError::Changed);
    }
    Rung {
        placing,
        mount: &mount,
        source: &source,
    }
    .place()
}

/// Continues with the source where it was stated already.
fn after_refused_rename(
    placing: &Placing<'_>,
    mount: &Mount<'_>,
    expected: Option<LocalFileIdentity>,
    error: Errno,
) -> ControlFlow<Result<Placed, PlaceError>, Option<EntryStat>> {
    match error {
        Errno::EXIST | Errno::NOTEMPTY => ControlFlow::Break(Err(PlaceError::Exists)),
        Errno::XDEV => ControlFlow::Break(Err(PlaceError::CrossDevice)),
        Errno::INVAL => match invalid_rename(placing, mount) {
            Ok(stat) => ControlFlow::Continue(Some(stat)),
            Err(error) => ControlFlow::Break(Err(error)),
        },
        error if is_unsupported(error) => {
            mount.learn(|capabilities| capabilities.no_replace = Some(false));
            ControlFlow::Continue(None)
        }
        error if may_have_lost_reply(error) => {
            ControlFlow::Break(resolve(placing, expected, error).map(|()| Placed::Renamed))
        }
        error => ControlFlow::Break(Err(PlaceError::Io(error))),
    }
}

/// `EINVAL` from NOREPLACE: a move into itself, a refused name, or the flag.
fn invalid_rename(placing: &Placing<'_>, mount: &Mount<'_>) -> Result<EntryStat, PlaceError> {
    let stat = stat_source(placing.fs, placing.from)?;
    let known = mount.capabilities().no_replace;
    if moves_into_itself(placing, &stat, known) {
        return Err(PlaceError::IntoItself);
    }
    let no_replace = match known {
        None => mount
            .probe(!stat.is_directory(), false)
            .ok()
            .and_then(|_| mount.capabilities().no_replace),
        known => known,
    };
    if no_replace == Some(true) {
        return Err(PlaceError::InvalidName);
    }
    Ok(stat)
}

/// Once NOREPLACE is known missing, the folder rung's own rename or the
/// caller's copy guard catches a move into itself.
fn moves_into_itself(placing: &Placing<'_>, stat: &EntryStat, known: Option<bool>) -> bool {
    if !stat.is_directory() || known == Some(false) {
        return false;
    }
    contains(placing.fs, stat.identity, placing.to.dir) != Some(false)
}

/// One placement past the first rung, of a source already stated.
struct Rung<'a> {
    placing: Placing<'a>,
    mount: &'a Mount<'a>,
    source: &'a EntryStat,
}

impl Rung<'_> {
    fn place(&self) -> Result<Placed, PlaceError> {
        let directory = self.source.is_directory();
        let placed = if directory {
            self.directory()
        } else {
            self.non_directory()
        };
        match placed {
            Err(PlaceError::Unsupported) if self.mount.allows_reserved_name(directory) => {
                self.over_reserved_name(directory)
            }
            placed => placed,
        }
    }

    fn over_reserved_name(&self, directory: bool) -> Result<Placed, PlaceError> {
        if directory {
            return self.folder_after_check();
        }
        if self.mount.renames_over(self.placing.to.name) {
            return self.over_reserved_file();
        }
        Err(PlaceError::Unsupported)
    }

    fn non_directory(&self) -> Result<Placed, PlaceError> {
        if self.mount.capabilities().hard_links != Some(false)
            && let Some(placed) = self.link()?
        {
            return Ok(placed);
        }
        self.recreate_symlink()
    }

    /// `None` when the next rung should try.
    fn link(&self) -> Result<Option<Placed>, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        match fs.link(from, to) {
            Ok(()) => {
                if self.link_holds_source()? {
                    return self.finish_link().map(Some);
                }
                Ok(None)
            }
            Err(Errno::EXIST) => {
                if link_reply_was_lost(fs, to, self.source) {
                    self.finish_link().map(Some)
                } else {
                    Err(PlaceError::Exists)
                }
            }
            Err(error) => self.link_failed(error),
        }
    }

    fn link_failed(&self, error: Errno) -> Result<Option<Placed>, PlaceError> {
        match error {
            Errno::PERM | Errno::ACCESS => self.link_refused(error).map(|()| None),
            Errno::MLINK => Ok(None),
            Errno::XDEV => Err(PlaceError::CrossDevice),
            Errno::INVAL => Err(PlaceError::InvalidName),
            error if is_unsupported(error) => {
                self.mount
                    .learn(|capabilities| capabilities.hard_links = Some(false));
                Ok(None)
            }
            error if may_have_lost_reply(error) => self.link_reply_lost(error).map(Some),
            error => Err(PlaceError::Io(error)),
        }
    }

    fn link_refused(&self, error: Errno) -> Result<(), PlaceError> {
        if exists(self.placing.fs, self.placing.to)? {
            return Err(PlaceError::Exists);
        }
        // Linking can be refused for this item alone (protected_hardlinks) or by
        // an SFTP server without a link extension (EACCES), so only a probe
        // teaches the mount anything; a folder that refuses the probe's own
        // file refused the link too.
        if self.mount.capabilities().hard_links.is_none()
            && self.mount.probe(true, false) == Err(Errno::ACCESS)
        {
            return Err(PlaceError::Io(error));
        }
        Ok(())
    }

    fn link_reply_lost(&self, error: Errno) -> Result<Placed, PlaceError> {
        match self.placing.fs.stat(self.placing.to, true) {
            Ok(target) if target.identity == self.source.identity => self.finish_link(),
            Ok(_) | Err(Errno::NOENT) => Err(PlaceError::Io(error)),
            Err(_) => Err(PlaceError::Unconfirmed),
        }
    }

    /// Only the source's identity at the new name proves a link; a link count can
    /// be stale, or always 1 on sshfs. Another identity there is a racer's item,
    /// or a mount where each link reports its own inode number (libfuse without
    /// `use_ino`, cifs without server inode numbers). Only a probe on Strata's own
    /// names tells them apart; on such a mount a link that matches the source's
    /// type, mode, size and time is removed by the identity it reported, and links
    /// are not used again.
    fn link_holds_source(&self) -> Result<bool, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        let source = self.source;
        let linked = fs.stat(to, true).map_err(|_| PlaceError::Unconfirmed)?;
        if linked.identity == source.identity {
            return Ok(true);
        }
        let current = match fs.stat(from, true) {
            Ok(current) if current.identity == source.identity => current,
            Ok(_) => return Err(PlaceError::Changed),
            Err(_) => return Err(PlaceError::Unconfirmed),
        };
        let alike = (linked.kind, linked.mode, linked.size, linked.mtime)
            == (current.kind, current.mode, current.size, current.mtime);
        let links_unusable = self
            .mount
            .probe(true, false)
            .is_ok_and(|probed| probed.hard_links == Some(false));
        if !links_unusable || !alike {
            return Err(PlaceError::Unconfirmed);
        }
        let removed = remove_if_unchanged(
            fs,
            to,
            linked.identity,
            linked.kept_attributes(),
            AtFlags::empty(),
        );
        if removed {
            Ok(false)
        } else {
            Err(PlaceError::Unconfirmed)
        }
    }

    fn finish_link(&self) -> Result<Placed, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        let source = self.source;
        let current = match fs.stat(from, true) {
            Ok(current) => current,
            Err(Errno::NOENT) => return Ok(Placed::Linked),
            Err(_) => return Err(PlaceError::Unconfirmed),
        };
        if current.identity != source.identity {
            return Err(PlaceError::Changed);
        }
        match fs.unlink(from, AtFlags::empty()) {
            Ok(()) => Ok(Placed::Linked),
            Err(Errno::NOENT) => {
                resolve(&self.placing, Some(source.identity), Errno::NOENT).map(|()| Placed::Linked)
            }
            Err(error) => {
                if fs
                    .stat(to, true)
                    .is_ok_and(|target| target.identity == source.identity && target.nlink >= 2)
                {
                    let _ = fs.unlink(to, AtFlags::empty());
                }
                Err(PlaceError::Io(error))
            }
        }
    }

    fn recreate_symlink(&self) -> Result<Placed, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        if self.source.kind != FileType::Symlink {
            return Err(PlaceError::Unsupported);
        }
        let link = fs.readlink(from).map_err(PlaceError::Io)?;
        match fs.symlink(&link, to) {
            Ok(()) => {}
            Err(Errno::EXIST) => return Err(PlaceError::Exists),
            Err(Errno::PERM | Errno::ACCESS) if exists(fs, to)? => return Err(PlaceError::Exists),
            Err(Errno::PERM | Errno::ACCESS) => return Err(PlaceError::Unsupported),
            Err(Errno::INVAL) => return Err(PlaceError::InvalidName),
            Err(error) => return Err(PlaceError::Io(error)),
        }
        let created = fs.stat(to, false).map_err(|_| PlaceError::Unconfirmed)?;
        let remove_created = || {
            remove_if_unchanged(
                fs,
                to,
                created.identity,
                created.kept_attributes(),
                AtFlags::empty(),
            )
        };
        match fs.stat(from, true) {
            Ok(current) if current.identity == self.source.identity => {
                match fs.unlink(from, AtFlags::empty()) {
                    Ok(()) | Err(Errno::NOENT) => Ok(Placed::RecreatedSymlink),
                    Err(error) => {
                        remove_created();
                        Err(PlaceError::Io(error))
                    }
                }
            }
            Err(Errno::NOENT) => Ok(Placed::RecreatedSymlink),
            _ => {
                remove_created();
                Err(PlaceError::Changed)
            }
        }
    }

    fn directory(&self) -> Result<Placed, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        if !self.mount.renames_over(to.name) {
            return if self.mount.capabilities().no_replace == Some(false) {
                self.folder_after_check()
            } else {
                Err(PlaceError::Unsupported)
            };
        }
        if self.mount.capabilities().rename_over_empty_directory == Some(false) {
            return Err(PlaceError::Unsupported);
        }
        self.make_placeholder()?;
        let placeholder = fs.stat(to, false).map_err(PlaceError::Io)?.identity;
        match fs.rename(from, to, RenameFlags::empty()) {
            Ok(()) => Ok(Placed::ReservedDirectory),
            Err(error) => self.placeholder_kept(placeholder, error),
        }
    }

    fn make_placeholder(&self) -> Result<(), PlaceError> {
        let Placing { fs, to, .. } = self.placing;
        match fs.mkdir(to, Mode::from_raw_mode(0o700)) {
            Ok(()) => Ok(()),
            Err(Errno::EXIST) => Err(PlaceError::Exists),
            Err(Errno::PERM) if exists(fs, to)? => Err(PlaceError::Exists),
            Err(Errno::INVAL) => Err(PlaceError::InvalidName),
            // A lost reply leaves an empty folder that cannot be told apart from a racer's.
            Err(error) if may_have_lost_reply(error) => Err(if exists(fs, to)? {
                PlaceError::Exists
            } else {
                PlaceError::Io(error)
            }),
            Err(error) => Err(PlaceError::Io(error)),
        }
    }

    /// The rename over the placeholder failed with `error`.
    fn placeholder_kept(
        &self,
        placeholder: LocalFileIdentity,
        error: Errno,
    ) -> Result<Placed, PlaceError> {
        let Placing { fs, to, .. } = self.placing;
        match error {
            Errno::NOTEMPTY | Errno::EXIST | Errno::PERM => {
                self.renaming_over_refused(placeholder, error)
            }
            Errno::NOTDIR | Errno::ISDIR => Err(PlaceError::Exists),
            Errno::INVAL => {
                remove_placeholder(fs, to, placeholder);
                Err(self.invalid_or_into_itself())
            }
            Errno::XDEV => {
                remove_placeholder(fs, to, placeholder);
                Err(PlaceError::CrossDevice)
            }
            error if may_have_lost_reply(error) => {
                if self.mount.numbers_kept() && remove_placeholder(fs, to, placeholder) {
                    Err(PlaceError::Io(error))
                } else {
                    self.moved_despite_lost_reply(Placed::ReservedDirectory)
                }
            }
            error => {
                remove_placeholder(fs, to, placeholder);
                Err(PlaceError::Io(error))
            }
        }
    }

    fn renaming_over_refused(
        &self,
        placeholder: LocalFileIdentity,
        error: Errno,
    ) -> Result<Placed, PlaceError> {
        let Placing { fs, to, .. } = self.placing;
        placeholder_still_empty(fs, to, placeholder)?;
        let _ = fs.unlink(to, AtFlags::REMOVEDIR);
        if error != Errno::PERM {
            self.mount
                .learn(|capabilities| capabilities.rename_over_empty_directory = Some(false));
        }
        Err(PlaceError::Unsupported)
    }

    fn invalid_or_into_itself(&self) -> PlaceError {
        let Placing { fs, to, .. } = self.placing;
        if contains(fs, self.source.identity, to.dir) == Some(false) {
            PlaceError::InvalidName
        } else {
            PlaceError::IntoItself
        }
    }

    fn over_reserved_file(&self) -> Result<Placed, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        let reserved = reserve_file(fs, to)?;
        if !still_reserved(fs, to, reserved)? {
            return Err(PlaceError::Exists);
        }
        let error = match fs.rename(from, to, RenameFlags::empty()) {
            Ok(()) => return Ok(Placed::OverReservedFile),
            Err(error) => error,
        };
        if may_have_lost_reply(error) {
            if self.mount.numbers_kept() && remove_reserved(fs, to, reserved) {
                return Err(PlaceError::Io(error));
            }
            return self.moved_despite_lost_reply(Placed::OverReservedFile);
        }
        let removed = remove_reserved(fs, to, reserved);
        Err(match error {
            Errno::INVAL => PlaceError::InvalidName,
            Errno::XDEV => PlaceError::CrossDevice,
            // SFTP without `posix-rename@openssh.com` refuses to rename over any file.
            Errno::PERM if removed => PlaceError::Unsupported,
            error => PlaceError::Io(error),
        })
    }

    /// Decides a rename whose reply may have been lost without removing anything:
    /// libfuse without `use_ino` keeps a name's inode number then, so a
    /// placeholder's number at `to` can belong to the moved item. Where numbers
    /// are stable, a placeholder still in place proves the rename never happened,
    /// and the rungs remove it before coming here.
    fn moved_despite_lost_reply(&self, placed: Placed) -> Result<Placed, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        let gone = matches!(fs.stat(from, true), Err(Errno::NOENT));
        if gone
            && fs
                .stat(to, true)
                .is_ok_and(|target| target.identity == self.source.identity)
        {
            Ok(placed)
        } else {
            Err(PlaceError::Unconfirmed)
        }
    }

    fn folder_after_check(&self) -> Result<Placed, PlaceError> {
        let Placing { fs, from, to } = self.placing;
        if exists(fs, to)? {
            return Err(PlaceError::Exists);
        }
        match fs.rename(from, to, RenameFlags::empty()) {
            Ok(()) => Ok(Placed::AfterCheck),
            Err(Errno::EXIST | Errno::NOTEMPTY | Errno::NOTDIR | Errno::ISDIR) => {
                Err(PlaceError::Exists)
            }
            Err(Errno::PERM) if exists(fs, to)? => Err(PlaceError::Exists),
            Err(Errno::INVAL) => Err(self.invalid_or_into_itself()),
            Err(Errno::XDEV) => Err(PlaceError::CrossDevice),
            Err(error) if may_have_lost_reply(error) => {
                self.moved_despite_lost_reply(Placed::AfterCheck)
            }
            Err(error) => Err(PlaceError::Io(error)),
        }
    }
}

fn placeholder_still_empty(
    fs: &dyn FsOps,
    to: Entry<'_>,
    placeholder: LocalFileIdentity,
) -> Result<(), PlaceError> {
    let current = match fs.stat(to, true) {
        Ok(current) => current,
        Err(Errno::NOENT) => return Err(PlaceError::Exists),
        Err(error) => return Err(PlaceError::Io(error)),
    };
    if current.identity != placeholder {
        return Err(PlaceError::Exists);
    }
    match fs.names(to) {
        Ok(names) if names.is_empty() => Ok(()),
        Ok(_) => Err(PlaceError::Exists),
        Err(error) => Err(PlaceError::Io(error)),
    }
}

fn reserve_file(fs: &dyn FsOps, to: Entry<'_>) -> Result<LocalFileIdentity, PlaceError> {
    match fs.create_exclusive(to, Mode::from_raw_mode(0o600)) {
        Ok(file) => Ok(fs.stat_fd(file.as_fd()).map_err(PlaceError::Io)?.identity),
        Err(Errno::PERM) if exists(fs, to)? => Err(PlaceError::Exists),
        Err(error) => Err(file_creation_error(error)),
    }
}

fn still_reserved(
    fs: &dyn FsOps,
    to: Entry<'_>,
    reserved: LocalFileIdentity,
) -> Result<bool, PlaceError> {
    match fs.stat(to, true) {
        Ok(current) => {
            let empty_file = current.kind == FileType::RegularFile && current.size == 0;
            Ok(current.identity == reserved && empty_file)
        }
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(PlaceError::Io(error)),
    }
}

fn remove_reserved(fs: &dyn FsOps, to: Entry<'_>, reserved: LocalFileIdentity) -> bool {
    still_reserved(fs, to, reserved) == Ok(true) && fs.unlink(to, AtFlags::empty()).is_ok()
}
