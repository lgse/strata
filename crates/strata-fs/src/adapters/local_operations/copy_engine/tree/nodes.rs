// SPDX-License-Identifier: MIT

use super::*;

/// sshfs reports a taken name as `EPERM`, and SMB a folder in a file's way
/// as `EISDIR`.
pub(in crate::adapters::local_operations) fn taken(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    error: Errno,
) -> bool {
    match error {
        Errno::EXIST | Errno::ISDIR => true,
        Errno::PERM => fs.stat(entry, false).is_ok(),
        _ => false,
    }
}

/// NFS creations are not idempotent, so a taken name may be this copy's own
/// entry whose reply was lost.
pub(in crate::adapters::local_operations::copy_engine) fn adopt(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    names: Option<&Names>,
    fresh: impl FnOnce(&EntryStat) -> bool,
) -> Result<LocalFileIdentity, Failure> {
    let ours = || {
        let names = names?;
        let stat = fs.stat(entry, true).ok()?;
        if !fresh(&stat) || !names.unclaimed(entry.name, &stat) {
            return None;
        }
        let listed = fs
            .names(Entry {
                dir: entry.dir,
                name: OsStr::new("."),
            })
            .ok()?
            .iter()
            .any(|name| name == entry.name);
        listed.then(|| {
            names.made(entry.name, stat.identity);
            stat.identity
        })
    };
    ours().ok_or(Failure::Place(PlaceError::Exists))
}

pub(super) fn claim_folder(
    fs: &dyn FsOps,
    entry: Entry<'_>,
    names: &Names,
) -> Result<LocalFileIdentity, Failure> {
    match fs.mkdir(entry, Mode::from_raw_mode(0o700)) {
        Ok(()) => {
            let identity = fs.stat(entry, false)?.identity;
            names.made(entry.name, identity);
            Ok(identity)
        }
        Err(error) if taken(fs, entry, error) => adopt(fs, entry, Some(names), |stat| {
            stat.is_directory() && fs.names(entry).is_ok_and(|names| names.is_empty())
        }),
        Err(error) => Err(creation_failure(fs, entry, error)),
    }
}

/// Claims a file's name without opening it for writing: vfat mounted with
/// `flush` waits at every such close, and the file job opens it once more.
/// On SMB, names made as nodes and looked up while other files were written
/// left the folder's rename waiting 35 s for Samba, so they are created.
pub(super) fn claim_file(
    copy: &Copy,
    entry: Entry<'_>,
    names: &Names,
    family: Family,
) -> Result<LocalFileIdentity, Failure> {
    let fs = copy.fs;
    let tries_node = family != Family::Smb && !copy.nodes_refused.load(Ordering::Relaxed);
    if tries_node && let Some(claimed) = claim_as_node(copy, entry, names) {
        return claimed;
    }
    match fs.create_readonly(entry, Mode::from_raw_mode(0o600)) {
        Ok(fd) => {
            let identity = fs.stat_fd(fd.as_fd())?.identity;
            fs.close(fd)?;
            names.made(entry.name, identity);
            Ok(identity)
        }
        Err(error) if taken(fs, entry, error) => adopt(fs, entry, Some(names), fresh_file),
        Err(error) => Err(creation_failure(fs, entry, error)),
    }
}

/// `None` where the mount refuses nodes, which it is then known for.
fn claim_as_node(
    copy: &Copy,
    entry: Entry<'_>,
    names: &Names,
) -> Option<Result<LocalFileIdentity, Failure>> {
    let fs = copy.fs;
    match fs.create_node(entry, FileType::RegularFile, Mode::from_raw_mode(0o600)) {
        Ok(()) => Some(fs.stat(entry, false).map_err(Failure::from).map(|stat| {
            names.made(entry.name, stat.identity);
            stat.identity
        })),
        Err(error) if taken(fs, entry, error) => Some(adopt(fs, entry, Some(names), fresh_file)),
        Err(error) if refuses_nodes(error) => {
            copy.nodes_refused.store(true, Ordering::Relaxed);
            None
        }
        Err(error) => Some(Err(creation_failure(fs, entry, error))),
    }
}

fn refuses_nodes(error: Errno) -> bool {
    matches!(error, Errno::PERM | Errno::NOSYS) || is_unsupported(error)
}

pub(super) fn fresh_file(stat: &EntryStat) -> bool {
    stat.kind == FileType::RegularFile && stat.size == 0 && stat.nlink == 1
}

/// A link the placement made again at its name is another entry, which holds
/// the copy only with the staged link's target.
pub(in crate::adapters::local_operations::copy_engine) fn made_again(
    fs: &dyn FsOps,
    at: Entry<'_>,
    link: Option<&OsStr>,
) -> manifest::Ownership {
    match (fs.stat(at, false), link) {
        (Ok(made), Some(link))
            if made.kind == FileType::Symlink
                && fs.readlink(at).is_ok_and(|current| current == link) =>
        {
            manifest::Ownership::of(&made)
        }
        _ => manifest::Ownership::unproven(),
    }
}

pub(in crate::adapters::local_operations::copy_engine) fn read_link(
    fs: &dyn FsOps,
    at: Entry<'_>,
) -> Result<OsString, Failure> {
    fs.readlink(at).map_err(|error| match error {
        Errno::INVAL => Failure::Changed,
        error => error.into(),
    })
}

/// Without `named`, the link is made from a stage, whose target was checked.
pub(in crate::adapters::local_operations::copy_engine) fn copy_link(
    copy: &Copy,
    (source, target): (Entry<'_>, Entry<'_>),
    mount: &Mount,
    named: Option<(&Names, &Site)>,
) -> Result<EntryStat, Failure> {
    let fs = copy.fs;
    let stat = copy.source_stat(source)?;
    if stat.kind != FileType::Symlink {
        return Err(Failure::Changed);
    }
    let link = read_link(fs, source)?;
    let names = named.map(|(names, _)| names);
    if named.is_some_and(|(_, site)| !copy.names.profiles.holds_link_to(site, &link)) {
        return Err(Failure::LinkRefused(Errno::ILSEQ));
    }
    if let Err(error) = fs.symlink(&link, target) {
        link_taken(fs, (target, &link), names, error)?;
    }
    if read_link(fs, target)? != link {
        return Err(Failure::Changed);
    }
    // sshfs sets a link's times on what it points to.
    if mount.family != Family::Fuse {
        set_link_times(fs, (target, &link), names.is_some(), &stat)?;
    }
    copy.add_file(stat.size);
    Ok(stat)
}

/// A link made by this copy whose reply was lost is adopted.
fn link_taken(
    fs: &dyn FsOps,
    (target, link): (Entry<'_>, &OsStr),
    names: Option<&Names>,
    error: Errno,
) -> Result<(), Failure> {
    if !taken(fs, target, error) {
        return Err(match creation_failure(fs, target, error) {
            Failure::Os(error) if refuses_node(FileType::Symlink, error) => {
                Failure::LinkRefused(error)
            }
            failure => failure,
        });
    }
    adopt(fs, target, names, |stat| {
        stat.kind == FileType::Symlink && fs.readlink(target).is_ok_and(|current| current == link)
    })?;
    Ok(())
}

/// A failure removes a named link this copy made.
fn set_link_times(
    fs: &dyn FsOps,
    (target, link): (Entry<'_>, &OsStr),
    named: bool,
    stat: &EntryStat,
) -> Result<(), Failure> {
    let Err(error) = best_effort(fs.set_link_times(target, stat.atime, stat.mtime)) else {
        return Ok(());
    };
    if named && fs.readlink(target).is_ok_and(|current| current == link) {
        let _ = fs.unlink(target, AtFlags::empty());
    }
    Err(error.into())
}

/// The source's xattrs are not copied: reading them would open it, which
/// lets a writer waiting on it go on.
pub(in crate::adapters::local_operations::copy_engine) fn copy_fifo(
    copy: &Copy,
    (source, target): (Entry<'_>, Entry<'_>),
    (parent, mount): (&EntryStat, &Mount),
    names: Option<&Names>,
) -> Result<(EntryStat, Option<LocalFileIdentity>), Failure> {
    let fs = copy.fs;
    let stat = copy.source_stat(source)?;
    if stat.kind != FileType::Fifo {
        return Err(Failure::Changed);
    }
    let identity = match fs.create_node(target, FileType::Fifo, Mode::from_raw_mode(0o600)) {
        Ok(()) => fs.stat(target, false)?.identity,
        Err(error) if taken(fs, target, error) => {
            adopt(fs, target, names, |stat| stat.kind == FileType::Fifo)?
        }
        Err(error) if refuses_node(FileType::Fifo, error) => return Ok((stat, None)),
        Err(error) => return Err(creation_failure(fs, target, error)),
    };
    if identity.kind != FileType::Fifo {
        return Err(Failure::Changed);
    }
    let finished = finish_pipe(copy, &stat, (target, identity), (parent, mount));
    // An unprovable public creation cannot authorize deleting a concurrent arrival.
    if finished.is_err() && names.is_some() {
        placement::remove_if_unchanged(fs, target, identity, None, AtFlags::empty());
    }
    finished?;
    copy.add_file(0);
    Ok((stat, Some(identity)))
}

pub(in crate::adapters::local_operations::copy_engine) fn finish_pipe(
    copy: &Copy,
    stat: &EntryStat,
    (target, identity): (Entry<'_>, LocalFileIdentity),
    (parent, mount): (&EntryStat, &Mount),
) -> Result<(), Failure> {
    let fs = copy.fs;
    let fd = match fs.open_fifo(target) {
        Ok(fd) => fd,
        Err(error) if is_fatal(error) => return Err(error.into()),
        Err(_) => return Ok(()),
    };
    let created = fs.stat_fd(fd.as_fd())?;
    if created.kind != FileType::Fifo || !mount.may_be(identity, &created) {
        return Err(Failure::Place(PlaceError::Changed));
    }
    Snapshot::of(stat).apply_to_pipe(
        fs,
        fd.as_fd(),
        &created,
        parent,
        mount,
        &copy.refused_xattrs,
    )?;
    best_effort(fs.close(fd)).map_err(Failure::Os)
}

/// How filesystems refuse a node of `kind` they cannot hold. Without named
/// pipes, vfat answers `EPERM`, exfat-fuse `EIO` and SMB `EOPNOTSUPP`;
/// without symbolic links, vfat answers `EPERM`, exfat-fuse `ENOSYS` and
/// SMB `EOPNOTSUPP`, and ntfs3 refuses a target too long with `EFBIG`.
pub(in crate::adapters::local_operations) fn refuses_node(kind: FileType, error: Errno) -> bool {
    let refused = match kind {
        FileType::Fifo => &[Errno::PERM, Errno::IO, Errno::NOSYS, Errno::INVAL][..],
        FileType::Symlink => &[Errno::PERM, Errno::NOSYS, Errno::FBIG],
        _ => return false,
    };
    refused.contains(&error) || is_unsupported(error)
}
