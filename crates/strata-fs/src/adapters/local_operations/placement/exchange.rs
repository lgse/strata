// SPDX-License-Identifier: MIT

use super::*;

/// `Ok(false)`: the filesystem rejected the flag, which is remembered for
/// the mount.
pub(in crate::adapters::local_operations) fn exchange(
    fs: &dyn FsOps,
    from: Entry<'_>,
    to: Entry<'_>,
) -> Result<bool, PlaceError> {
    match fs.rename(from, to, RenameFlags::EXCHANGE) {
        Ok(()) => Ok(true),
        Err(error) if error == Errno::INVAL || is_unsupported(error) => {
            Mount::new(fs, to.dir).learn(|capabilities| capabilities.exchange = Some(false));
            Ok(false)
        }
        Err(Errno::XDEV) => Err(PlaceError::CrossDevice),
        Err(error) => Err(PlaceError::Io(error)),
    }
}

pub(in crate::adapters::local_operations) enum Exchanged {
    Swapped(Originals),
    Refused,
}

pub(in crate::adapters::local_operations) fn exchange_through(
    fs: &dyn FsOps,
    stage: Entry<'_>,
    target: Entry<'_>,
    copy: LocalFileIdentity,
    attributes: Option<Attributes>,
) -> Result<Exchanged, PlaceError> {
    let denied = |error| matches!(error, PlaceError::Io(Errno::ACCESS | Errno::PERM));
    let originals = Originals::create(fs, target.dir)?;
    let parked = originals.entry(target.name);
    let copy = match move_aside(fs, stage, parked, copy, attributes) {
        Ok((copy, _)) => copy,
        Err(error) => {
            if error != PlaceError::Unconfirmed {
                let _ = originals.remove(fs, target.dir, &Deadline::default());
            }
            return if denied(error) {
                Ok(Exchanged::Refused)
            } else {
                Err(error)
            };
        }
    };
    let exchanged = exchange(fs, parked, target);
    if let Ok(true) = exchanged {
        return Ok(Exchanged::Swapped(originals));
    }
    place(fs, parked, stage, Some(copy))?;
    let _ = originals.remove(fs, target.dir, &Deadline::default());
    match exchanged {
        Err(error) if !denied(error) => Err(error),
        _ => Ok(Exchanged::Refused),
    }
}

/// Filesystems known to implement `RENAME_EXCHANGE` faithfully. FUSE,
/// network, overlay and the other FAT-family drivers are left out because
/// some silently overwrite instead.
const EXCHANGE_FILESYSTEMS: [u32; 8] = [
    0xEF53,      // ext2/3/4
    0x5846_5342, // XFS
    0x9123_683E, // btrfs
    0xF2F5_2010, // f2fs
    0x0102_1994, // tmpfs
    0xCA45_1A4E, // bcachefs
    0x2FC1_2FC1, // ZFS
    0x4D44,      // vfat
];

pub(in crate::adapters::local_operations) fn exchange_allowed(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
) -> bool {
    fs.statfs(dir)
        .is_some_and(|stat| EXCHANGE_FILESYSTEMS.contains(&(stat.kind as u32)))
        && Mount::new(fs, dir).capabilities().exchange != Some(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::adapters::local_operations) enum Displacement {
    Exchange,
    SetAside,
}

pub(in crate::adapters::local_operations) fn displacement_without_trash(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    directory: bool,
) -> Result<Option<Displacement>, PlaceError> {
    if exchange_allowed(fs, dir) {
        return Ok(Some(Displacement::Exchange));
    }
    set_aside_if_safe(fs, dir, directory)
}

pub(in crate::adapters::local_operations) fn set_aside_if_safe(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    directory: bool,
) -> Result<Option<Displacement>, PlaceError> {
    Ok((can_set_aside(fs, dir, directory)? == Feasibility::Yes).then_some(Displacement::SetAside))
}
