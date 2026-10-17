// SPDX-License-Identifier: MIT

//! The data ladder: one clone, then `copy_file_range` in chunks, then reads
//! and writes, leaving out the holes of sparse local files. Progress and
//! cancellation are checked between calls.

use std::{
    cell::OnceCell,
    os::fd::BorrowedFd,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use ring::digest;
use rustix::io::Errno;

use super::{Failure, Mount, is_fatal, is_unsupported};
use crate::adapters::{
    local_operations::{
        fs_ops::{EntryStat, FsOps},
        placement::ENOTSUPP,
    },
    volume::{Family, MountKey, learn_mount_capability},
};

const MIB: usize = 1 << 20;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Network {
    Nfs,
    Smb,
    Fuse,
}

fn network(family: Family) -> Option<Network> {
    match family {
        Family::Nfs | Family::Nfs4 => Some(Network::Nfs),
        Family::Smb => Some(Network::Smb),
        Family::Fuse => Some(Network::Fuse),
        _ => None,
    }
}

#[cfg(test)]
static TEST_KERNEL: std::sync::Mutex<Option<(u32, u32, u32)>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(in crate::adapters::local_operations) fn set_kernel_for_test(release: Option<(u32, u32, u32)>) {
    *super::lock(&TEST_KERNEL) = release;
}

pub(super) fn kernel() -> (u32, u32, u32) {
    #[cfg(test)]
    if let Some(release) = *super::lock(&TEST_KERNEL) {
        return release;
    }
    static KERNEL: OnceLock<(u32, u32, u32)> = OnceLock::new();
    *KERNEL.get_or_init(|| {
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        let mut numbers = release
            .trim()
            .split(|character: char| !character.is_ascii_digit())
            .map(|number| number.parse().unwrap_or(0));
        (
            numbers.next().unwrap_or(0),
            numbers.next().unwrap_or(0),
            numbers.next().unwrap_or(0),
        )
    })
}

/// Whether the kernel has the client's fixes for stale caches after
/// server-side copies.
fn has_copy_fixes(network: Network) -> bool {
    let kernel = kernel();
    match network {
        Network::Nfs => kernel >= (5, 9, 0),
        Network::Fuse => kernel >= (5, 8, 0),
        // The December 2023 fixes for flushing, invalidation and file size:
        // 6.7, 6.6.7, and 6.1.68 with its backport regression fixed in 6.1.73.
        Network::Smb => {
            kernel >= (6, 7, 0)
                || ((6, 6, 7)..(6, 7, 0)).contains(&kernel)
                || ((6, 1, 73)..(6, 2, 0)).contains(&kernel)
        }
    }
}

pub(super) struct Route {
    target: MountKey,
    clone: bool,
    copy_range: Option<usize>,
    holes: bool,
}

impl Route {
    pub(super) fn new(
        fs: &dyn FsOps,
        target: &Mount,
        source_mount: Option<&Mount>,
        source: BorrowedFd<'_>,
        source_stat: &EntryStat,
        parent: &EntryStat,
    ) -> Self {
        // A folder may hold another mount, so a source whose mount is unknown
        // asks its own, once.
        let source_family = OnceCell::new();
        let source_family = || {
            *source_family.get_or_init(|| {
                source_mount.map_or_else(|| Mount::new(fs, source).family, |mount| mount.family)
            })
        };
        let target_network = network(target.family);
        let capabilities = target.capabilities();
        // It stops at the reported size, which virtual files leave at 0
        // whatever they hold.
        let copy_range = source_stat.size > 0
            && match target_network {
                None => source_stat.identity.device == parent.identity.device,
                Some(target_network) => {
                    has_copy_fixes(target_network)
                        && network(source_family()) == Some(target_network)
                }
            }
            && capabilities.copy_range != Some(false);
        Self {
            target: target.key,
            clone: source_stat.size > 0
                && capabilities.reflink != Some(false)
                && (target_network != Some(Network::Smb) || has_copy_fixes(Network::Smb)),
            // A larger NFS request becomes an asynchronous COPY that costs
            // seconds whatever its size.
            copy_range: copy_range.then_some(if target_network == Some(Network::Nfs) {
                2 * MIB
            } else {
                64 * MIB
            }),
            holes: sparse(source_stat) && stores_holes(target) && reports_holes(source_family()),
        }
    }
}

/// As `cp --sparse=auto` decides.
fn sparse(stat: &EntryStat) -> bool {
    stat.size >= 1 << 20 && stat.blocks.saturating_mul(512) < stat.size
}

/// FAT, exFAT and NTFS fill a skipped hole with zeros within one call, which
/// reports no progress and cannot be cancelled, through exfat-fuse and
/// ntfs-3g as in the kernel.
fn stores_holes(target: &Mount) -> bool {
    !matches!(target.family, Family::Fat | Family::Ntfs3) && target.fuse_format.is_none()
}

/// Some ZFS releases report holes over unsynced data, also through NFS.
fn reports_holes(family: Family) -> bool {
    matches!(
        family,
        Family::Ext | Family::Xfs | Family::Btrfs | Family::Tmpfs | Family::F2fs
    )
}

fn holes_unknown(error: Errno) -> bool {
    matches!(error, Errno::INVAL | Errno::OPNOTSUPP | ENOTSUPP)
}

#[derive(Clone, Copy)]
struct Ends<'a> {
    fs: &'a dyn FsOps,
    cancelled: &'a AtomicBool,
    source: BorrowedFd<'a>,
    target: BorrowedFd<'a>,
}

impl Ends<'_> {
    fn check_cancelled(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Failure::Cancelled);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Until {
    At(u64),
    End(u64),
}

impl Until {
    fn expected(self) -> u64 {
        match self {
            Self::At(offset) | Self::End(offset) => offset,
        }
    }
}

enum Ranged {
    Done,
    From(u64),
    Missing(u64),
}

enum Walked {
    Done,
    Dense(u64),
}

#[derive(Default)]
pub(super) struct Data {
    buffer: Vec<u8>,
    /// A small request took over a second, as on NFS with a small rsize.
    copy_range_slow: bool,
}

impl Data {
    /// The offset at which the copy found the source's end, where its own reads tell.
    #[expect(
        clippy::too_many_arguments,
        reason = "one ladder step per argument group, all borrowed from the job"
    )]
    pub(super) fn copy(
        &mut self,
        fs: &dyn FsOps,
        cancelled: &AtomicBool,
        target_mount: &Mount,
        route: &Route,
        (source, target): (BorrowedFd<'_>, BorrowedFd<'_>),
        size: u64,
        mut progress: impl FnMut(u64),
    ) -> Result<Option<u64>, Failure> {
        if route.clone {
            match fs.reflink(target, source) {
                Ok(()) => {
                    progress(size);
                    return Ok(None);
                }
                // Writeback may have reverted the page cache, so a fallback
                // could read wrong bytes.
                Err(error) if is_fatal(error) => return Err(error.into()),
                Err(error) => {
                    if is_unsupported(error) {
                        target_mount.learn(|capabilities| capabilities.reflink = Some(false));
                    }
                }
            }
        }
        let ends = Ends {
            fs,
            cancelled,
            source,
            target,
        };
        let mut offset = 0;
        if route.holes {
            match self.copy_data_ranges(ends, route, size, &mut progress)? {
                // Then whatever the source gained past `size`.
                Walked::Done => {
                    self.read_write(ends, size, Until::End(size), &mut progress, None)?;
                    return Ok(None);
                }
                Walked::Dense(from) => offset = from,
            }
        }
        self.copy_span(ends, route, offset, Until::End(size), &mut progress)
            .map(Some)
    }

    pub(super) fn copy_digested(
        &mut self,
        fs: &dyn FsOps,
        cancelled: &AtomicBool,
        (source, target): (BorrowedFd<'_>, BorrowedFd<'_>),
        size: u64,
        mut progress: impl FnMut(u64),
    ) -> Result<(digest::Digest, u64), Failure> {
        let ends = Ends {
            fs,
            cancelled,
            source,
            target,
        };
        let mut digest = digest::Context::new(&digest::SHA256);
        let end = self.read_write(ends, 0, Until::End(size), &mut progress, Some(&mut digest))?;
        Ok((digest.finish(), end))
    }

    fn copy_data_ranges(
        &mut self,
        ends: Ends<'_>,
        route: &Route,
        size: u64,
        progress: &mut impl FnMut(u64),
    ) -> Result<Walked, Failure> {
        let mut offset = 0;
        while offset < size {
            ends.check_cancelled()?;
            let data = match ends.fs.seek_data(ends.source, offset) {
                Ok(data) => data.clamp(offset, size),
                Err(Errno::NXIO) => size,
                Err(error) if holes_unknown(error) => return Ok(Walked::Dense(offset)),
                Err(error) => return Err(error.into()),
            };
            let hole = if data < size {
                match ends.fs.seek_hole(ends.source, data) {
                    Ok(hole) => hole.clamp(data, size),
                    Err(Errno::NXIO) => size,
                    Err(error) if holes_unknown(error) => return Ok(Walked::Dense(offset)),
                    Err(error) => return Err(error.into()),
                }
            } else {
                size
            };
            if data == 0 && hole == size {
                return Ok(Walked::Dense(0));
            }
            progress(data - offset);
            if data < hole {
                self.copy_span(ends, route, data, Until::At(hole), progress)?;
            }
            offset = hole;
        }
        // A source that shrank meanwhile ends where it ends now.
        let now = ends.fs.stat_fd(ends.source)?.size;
        ends.fs.set_len(ends.target, size.min(now))?;
        Ok(Walked::Done)
    }

    fn copy_span(
        &mut self,
        ends: Ends<'_>,
        route: &Route,
        mut offset: u64,
        until: Until,
        progress: &mut impl FnMut(u64),
    ) -> Result<u64, Failure> {
        if let Some(largest) = route.copy_range.filter(|_| !self.copy_range_slow) {
            match (
                self.copy_ranges(ends, (offset, until.expected()), largest, progress)?,
                until,
            ) {
                (Ranged::Done, Until::At(end)) => return Ok(end),
                // Then whatever the source gained past `size`.
                (Ranged::Done, Until::End(size)) => offset = size,
                (Ranged::From(copied), _) => offset = copied,
                (Ranged::Missing(copied), _) => {
                    learn_mount_capability(route.target, |capabilities| {
                        capabilities.copy_range = Some(false);
                    });
                    offset = copied;
                }
            }
        }
        self.read_write(ends, offset, until, progress, None)
    }

    fn copy_ranges(
        &mut self,
        ends: Ends<'_>,
        (mut offset, end): (u64, u64),
        largest: usize,
        progress: &mut impl FnMut(u64),
    ) -> Result<Ranged, Failure> {
        let mut chunk = MIB.min(largest);
        loop {
            ends.check_cancelled()?;
            let request =
                usize::try_from(end.saturating_sub(offset)).map_or(chunk, |left| left.min(chunk));
            let started = Instant::now();
            let copied = match ends
                .fs
                .copy_range(ends.source, offset, ends.target, offset, request)
            {
                Ok(copied) => copied,
                Err(Errno::INTR) => continue,
                Err(Errno::NOSYS) => return Ok(Ranged::Missing(offset)),
                Err(
                    Errno::XDEV
                    | Errno::OPNOTSUPP
                    | ENOTSUPP
                    | Errno::INVAL
                    | Errno::BADF
                    | Errno::TXTBSY
                    | Errno::PERM
                    | Errno::ACCESS
                    | Errno::NOENT,
                ) => return Ok(Ranged::From(offset)),
                Err(error) => return Err(error.into()),
            };
            let elapsed = started.elapsed();
            if copied == 0 {
                // An end before the size, which a read confirms.
                return Ok(Ranged::From(offset));
            }
            offset += copied as u64;
            progress(copied as u64);
            if offset >= end {
                return Ok(Ranged::Done);
            }
            if elapsed > Duration::from_secs(1) && chunk <= 2 * MIB {
                self.copy_range_slow = true;
                return Ok(Ranged::From(offset));
            }
            if elapsed > Duration::from_millis(250) {
                chunk = (chunk / 2).max(64 * 1024);
            } else if copied == chunk && elapsed < Duration::from_millis(100) {
                chunk = (chunk * 2).min(largest);
            }
        }
    }

    fn read_write(
        &mut self,
        ends: Ends<'_>,
        mut offset: u64,
        until: Until,
        progress: &mut impl FnMut(u64),
        mut digest: Option<&mut digest::Context>,
    ) -> Result<u64, Failure> {
        // Sized by what is left, so copying a small file maps no large buffer.
        let wanted = usize::try_from(until.expected().saturating_sub(offset))
            .map_or(MIB, |left| left.clamp(64 * 1024, MIB));
        if self.buffer.len() < wanted {
            self.buffer = vec![0; wanted];
        }
        loop {
            ends.check_cancelled()?;
            let limit = match until {
                Until::At(end) => usize::try_from(end.saturating_sub(offset))
                    .map_or(self.buffer.len(), |left| left.min(self.buffer.len())),
                Until::End(_) => self.buffer.len(),
            };
            if limit == 0 {
                return Ok(offset);
            }
            let read = match ends
                .fs
                .read_at(ends.source, &mut self.buffer[..limit], offset)
            {
                Ok(read) => read,
                Err(Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            };
            if read == 0 {
                return Ok(offset);
            }
            if let Some(digest) = &mut digest {
                digest.update(&self.buffer[..read]);
            }
            let mut written = 0;
            while written < read {
                match ends.fs.write_at(
                    ends.target,
                    &self.buffer[written..read],
                    offset + written as u64,
                ) {
                    Ok(0) => return Err(Errno::IO.into()),
                    Ok(count) => written += count,
                    Err(Errno::INTR) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            offset += read as u64;
            progress(read as u64);
        }
    }
}
