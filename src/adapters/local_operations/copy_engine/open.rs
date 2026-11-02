// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct Opened {
    pub(super) fd: SourceFd,
    pub(super) stat: EntryStat,
}

pub(super) struct SourceFd {
    pub(super) fd: Option<OwnedFd>,
    pub(super) held: Option<OneOpen>,
}

impl SourceFd {
    pub(super) fn new(fd: OwnedFd) -> Self {
        Self {
            fd: Some(fd),
            held: None,
        }
    }
}

impl AsFd for SourceFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_ref().expect("open until dropped").as_fd()
    }
}

impl Drop for SourceFd {
    fn drop(&mut self) {
        // Closed before `held` is released, as the fields drop after this.
        drop(self.fd.take());
    }
}

pub(super) fn open_failure(name: &OsStr, error: Errno) -> Failure {
    match error {
        Errno::NOENT | Errno::NOTDIR | Errno::LOOP | Errno::STALE | Errno::NXIO | Errno::NODEV => {
            Failure::Error(io_error(rust_i18n::t!(
                "%{name} changed while it was being copied: %{error}",
                name = name.to_string_lossy(),
                error = os_error_detail(error)
            )))
        }
        error => Failure::Os(error).named(name),
    }
}

pub(super) fn open_file(
    fs: &dyn FsOps,
    at: Entry<'_>,
    cancelled: &AtomicBool,
) -> Result<Opened, Failure> {
    let opened = match fs.open_source(at, true) {
        Err(Errno::INVAL) => open_after_deferred_close(fs, at, cancelled),
        opened => opened.map_err(|error| open_failure(at.name, error)),
    };
    match opened? {
        Some((fd, stat)) => Ok(Opened {
            fd: SourceFd::new(fd),
            stat,
        }),
        None => Err(Failure::Changed),
    }
}

/// After this machine writes a file, the SMB client keeps its handle open
/// for `closetimeo` and refuses an open of another name of the file with
/// `EINVAL` meanwhile, so the open is tried again until then.
pub(super) fn open_after_deferred_close(
    fs: &dyn FsOps,
    at: Entry<'_>,
    cancelled: &AtomicBool,
) -> Result<Option<(OwnedFd, EntryStat)>, Failure> {
    let Some(deferred) = Mount::new(fs, at.dir).deferred_close else {
        return Err(open_failure(at.name, Errno::INVAL));
    };
    let mut wait = deferred_close::Wait::after(deferred);
    loop {
        match wait.pause(&|| cancelled.load(Ordering::Acquire)) {
            Pause::Retry => {}
            Pause::Spent => return Err(open_failure(at.name, Errno::INVAL)),
            Pause::Cancelled => return Err(Failure::Cancelled),
        }
        match fs.open_source(at, true) {
            Err(Errno::INVAL) => {}
            opened => return opened.map_err(|error| open_failure(at.name, error)),
        }
    }
}

pub(super) fn open_one_name(
    fs: &dyn FsOps,
    at: Entry<'_>,
    seen: &EntryStat,
    cancelled: &AtomicBool,
) -> Result<Opened, Failure> {
    let held = OneOpen::wait(seen, cancelled)?;
    let mut source = open_file(fs, at, cancelled)?;
    source.fd.held = Some(held);
    if source.stat.identity != seen.identity {
        return Err(Failure::Changed);
    }
    Ok(source)
}

/// Files on SMB sources that a copy has a name of open, by device and
/// inode: the SMB client takes one lease per file and refuses an open of
/// another name meanwhile, or a Samba server defers it forever. Link counts
/// the client took from a listing say 1, so every file is held.
pub(super) static OPEN_ON_SMB: Mutex<BTreeSet<(u64, u64)>> = Mutex::new(BTreeSet::new());
pub(super) static OPEN_ON_SMB_CLOSED: Condvar = Condvar::new();

pub(super) struct OneOpen((u64, u64));

impl OneOpen {
    pub(super) fn wait(seen: &EntryStat, cancelled: &AtomicBool) -> Result<Self, Failure> {
        let key = (seen.identity.device, seen.identity.inode);
        let mut open = lock(&OPEN_ON_SMB);
        while !open.insert(key) {
            if cancelled.load(Ordering::Acquire) {
                return Err(Failure::Cancelled);
            }
            open = OPEN_ON_SMB_CLOSED
                .wait_timeout(open, Duration::from_millis(50))
                .map_or_else(|error| error.into_inner().0, |(open, _)| open);
        }
        Ok(Self(key))
    }
}

impl Drop for OneOpen {
    fn drop(&mut self) {
        lock(&OPEN_ON_SMB).remove(&self.0);
        OPEN_ON_SMB_CLOSED.notify_all();
    }
}
