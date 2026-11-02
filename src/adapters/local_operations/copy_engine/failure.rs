// SPDX-License-Identifier: MIT

use super::*;

pub(super) enum Failure {
    Cancelled,
    Os(Errno),
    NameRefused(Errno),
    LinkRefused(Errno),
    Place(PlaceError),
    Changed,
    NotCopyable,
    Error(glib::Error),
}

impl From<Errno> for Failure {
    fn from(error: Errno) -> Self {
        Self::Os(error)
    }
}

impl From<PlaceError> for Failure {
    fn from(error: PlaceError) -> Self {
        match error {
            PlaceError::Io(errno) if !error.refuses_name() => Self::Os(errno),
            error => Self::Place(error),
        }
    }
}

impl Failure {
    pub(super) fn named(self, name: &OsStr) -> Self {
        Self::Error(self.into_error(name))
    }

    pub(super) fn refuses_name(&self) -> bool {
        match self {
            Self::NameRefused(_) => true,
            Self::Place(error) => error.refuses_name(),
            _ => false,
        }
    }

    pub(super) fn into_error(self, name: &OsStr) -> glib::Error {
        let name = name.to_string_lossy();
        match self {
            Self::Cancelled => cancelled_local_operation(),
            Self::Os(error) => {
                let message = rust_i18n::t!(
                    "Could not copy %{name}: %{error}",
                    name = name,
                    error = os_error_detail(error)
                );
                match error {
                    Errno::NOSPC | Errno::DQUOT => {
                        glib::Error::new(gio::IOErrorEnum::NoSpace, &message)
                    }
                    _ => io_error(message),
                }
            }
            Self::NameRefused(error) => refused_name(&name, error),
            Self::LinkRefused(error) => glib::Error::new(
                Unheld::Symlink,
                &rust_i18n::t!(
                    "Could not copy %{name}: %{error}",
                    name = name,
                    error = os_error_detail(error)
                ),
            ),
            Self::Place(error @ PlaceError::Io(errno)) if error.refuses_name() => {
                refused_name(&name, errno)
            }
            Self::Place(PlaceError::IntoItself) => placement_error(PlaceError::IntoItself),
            Self::Place(PlaceError::Changed) => io_error(rust_i18n::t!(
                "%{name} changed while the operation was in progress",
                name = name
            )),
            Self::Place(error) => glib::Error::new(
                placement_error_kind(error),
                &rust_i18n::t!(
                    "Could not copy %{name}: %{error}",
                    name = name,
                    error = crate::services::error_detail(placement_reason(error))
                ),
            ),
            Self::Changed => io_error(rust_i18n::t!(
                "%{name} changed while it was being copied",
                name = name
            )),
            Self::NotCopyable => io_error(rust_i18n::t!(
                "Cannot copy %{name}: it is not a regular file, directory, or symbolic link",
                name = name
            )),
            Self::Error(error) => error,
        }
    }
}

pub(super) fn refused_name(name: &str, error: Errno) -> glib::Error {
    let kind = if error == Errno::NAMETOOLONG {
        gio::IOErrorEnum::FilenameTooLong
    } else {
        gio::IOErrorEnum::InvalidFilename
    };
    let message = rust_i18n::t!(
        "Could not copy %{name}: %{error}",
        name = name,
        error = crate::services::error_detail(refusal_reason(error))
    );
    glib::Error::new(kind, &message)
}

pub(in crate::adapters::local_operations) fn refusal_reason(error: Errno) -> String {
    if error == Errno::NAMETOOLONG {
        os_error_text(error)
    } else {
        crate::i18n::tr(INVALID_NAME_REASON)
    }
}

/// SMB and exfat-fuse refuse a name with `ENOENT` in a folder that still exists.
pub(super) fn creation_failure(fs: &dyn FsOps, at: Entry<'_>, error: Errno) -> Failure {
    if refuses_name(fs, at.dir, error) {
        Failure::NameRefused(error)
    } else {
        Failure::Os(error)
    }
}

pub(in crate::adapters::local_operations) fn refuses_name(
    fs: &dyn FsOps,
    dir: BorrowedFd<'_>,
    error: Errno,
) -> bool {
    match error {
        Errno::INVAL | Errno::ILSEQ | Errno::NAMETOOLONG | Errno::BADMSG => true,
        Errno::NOENT => fs.stat_fd(dir).is_ok_and(|folder| folder.nlink > 0),
        _ => false,
    }
}

pub(super) fn is_fatal(error: Errno) -> bool {
    matches!(
        error,
        Errno::IO
            | Errno::STALE
            | Errno::NOTCONN
            | Errno::CONNABORTED
            | Errno::CONNRESET
            | Errno::TIMEDOUT
            | Errno::HOSTDOWN
            | Errno::HOSTUNREACH
            | Errno::NETDOWN
            | Errno::NETUNREACH
    )
}

pub(super) fn best_effort(result: Result<(), Errno>) -> Result<(), Errno> {
    match result {
        Err(error) if is_fatal(error) => Err(error),
        _ => Ok(()),
    }
}

pub(super) fn is_unsupported(error: Errno) -> bool {
    matches!(error, Errno::OPNOTSUPP | Errno::NOTTY | ENOTSUPP)
}

pub(in crate::adapters::local_operations) fn holds_no_links(error: Errno) -> bool {
    error == Errno::NOSYS || is_unsupported(error)
}

pub(super) fn remove_own<'a>(
    fs: &dyn FsOps,
    entry: Entry<'a>,
    made: impl FnOnce(&EntryStat) -> bool,
    remove: impl FnOnce(Entry<'a>) -> Result<(), leftovers::Refused>,
) -> Result<(), glib::Error> {
    match fs.stat(entry, true) {
        Ok(current) if made(&current) => match remove(entry) {
            Ok(()) | Err(leftovers::Refused::Os(Errno::NOENT)) => Ok(()),
            Err(leftovers::Refused::Changed(_)) => Err(placement_error(PlaceError::Changed)),
            Err(refused) => Err(io_error(os_error_detail(refused.errno()))),
        },
        Ok(_) => Err(placement_error(PlaceError::Changed)),
        Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(io_error(os_error_detail(error))),
    }
}
