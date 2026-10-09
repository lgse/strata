// SPDX-License-Identifier: MIT

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Longest chain of symlinks [`atomic_write_config`] follows before failing
/// the way a symlink loop would.
const MAX_LINK_HOPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkPolicy {
    /// Never follow a final-component symlink (#15): caches, state, installers.
    Reject,
    /// User-edited configuration that dotfiles setups often symlink: follow a
    /// chain of links owned by the effective user to a regular file they own.
    FollowUserOwned,
}

struct Destination {
    /// Where the temporary file is renamed to: the resolved target for links.
    path: PathBuf,
    /// Permission bits of the existing destination, kept by config writes.
    mode: Option<u32>,
}

/// Why an atomic write refused its destination.
/// `Display` is English for logs and tests; [`Self::message`] is for the UI.
#[derive(Debug)]
pub(crate) enum DestinationError {
    NonRegular { path: PathBuf },
    MissingTarget { link: PathBuf, target: PathBuf },
    TooManyLinks { path: PathBuf },
    NotOwned { symlink: bool, path: PathBuf },
    /// A link's target may live elsewhere, so the error names the file that failed.
    AtTarget { target: PathBuf, error: io::Error },
}

impl fmt::Display for DestinationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonRegular { path } => write!(
                formatter,
                "refusing to replace non-regular destination {}",
                path.display()
            ),
            Self::MissingTarget { link, target } => write!(
                formatter,
                "symlink {} points to missing target {}",
                link.display(),
                target.display()
            ),
            Self::TooManyLinks { path } => write!(
                formatter,
                "too many levels of symbolic links resolving {}",
                path.display()
            ),
            Self::NotOwned { symlink, path } => write!(
                formatter,
                "refusing to write through {} {} owned by another user",
                if *symlink { "symlink" } else { "file" },
                path.display()
            ),
            Self::AtTarget { target, error } => {
                write!(formatter, "symlink target {}: {error}", target.display())
            }
        }
    }
}

impl std::error::Error for DestinationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AtTarget { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl DestinationError {
    /// Localized, standalone text; [`crate::services::io_error_message`] uses it.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::NonRegular { path } => rust_i18n::t!(
                "The destination “%{path}” is not a regular file",
                path = path.display()
            )
            .into_owned(),
            Self::MissingTarget { link, target } => rust_i18n::t!(
                "The symlink “%{link}” points to a missing target “%{target}”",
                link = link.display(),
                target = target.display()
            )
            .into_owned(),
            Self::TooManyLinks { path } => rust_i18n::t!(
                "There are too many levels of symbolic links in “%{path}”",
                path = path.display()
            )
            .into_owned(),
            Self::NotOwned {
                symlink: true,
                path,
            } => rust_i18n::t!(
                "The symlink “%{path}” belongs to another user",
                path = path.display()
            )
            .into_owned(),
            Self::NotOwned {
                symlink: false,
                path,
            } => rust_i18n::t!(
                "The file “%{path}” belongs to another user",
                path = path.display()
            )
            .into_owned(),
            Self::AtTarget { target, error } => rust_i18n::t!(
                "Could not write the symlink target “%{path}”: %{error}",
                path = target.display(),
                error = crate::services::io_error_detail(error)
            )
            .into_owned(),
        }
    }

    fn into_io(self, kind: io::ErrorKind) -> io::Error {
        io::Error::new(kind, self)
    }
}

pub(crate) fn config_directory() -> PathBuf {
    gtk::glib::user_config_dir().join("strata")
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    atomic_write_with(path, LinkPolicy::Reject, |file| file.write_all(contents))
}

/// [`atomic_write`] for configuration the user may manage as dotfiles. A
/// symlinked destination is written through to its target, which is replaced
/// in its own directory while the links stay untouched; every link and the
/// target must belong to the effective user. An existing destination keeps
/// its permission bits, and a new one is created owner-only.
pub(crate) fn atomic_write_config(path: &Path, contents: &[u8]) -> io::Result<()> {
    atomic_write_with(path, LinkPolicy::FollowUserOwned, |file| {
        file.write_all(contents)
    })
}

fn atomic_write_with(
    path: &Path,
    policy: LinkPolicy,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let destination = resolve_destination(path, policy)?;
    let parent = destination
        .path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    let at_target = |error: io::Error| {
        if destination.path == path {
            error
        } else {
            let kind = error.kind();
            DestinationError::AtTarget {
                target: destination.path.clone(),
                error,
            }
            .into_io(kind)
        }
    };
    let (temporary_path, mut file) = create_temporary_file(parent).map_err(at_target)?;

    let result = (|| {
        write(&mut file)?;
        file.flush()?;
        if policy == LinkPolicy::FollowUserOwned
            && let Some(mode) = destination.mode
        {
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.sync_all()?;
        drop(file);
        // `rename` never follows a link, so a target swapped for one after this
        // check is replaced itself rather than written through.
        validate_destination(&destination.path)?;
        fs::rename(&temporary_path, &destination.path).map_err(at_target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    result
}

fn resolve_destination(path: &Path, policy: LinkPolicy) -> io::Result<Destination> {
    let owner = rustix::process::geteuid().as_raw();
    let mut current = path.to_path_buf();
    let mut hops = 0;
    loop {
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound && hops == 0 => {
                return Ok(Destination {
                    path: current,
                    mode: None,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(DestinationError::MissingTarget {
                    link: path.to_path_buf(),
                    target: current,
                }
                .into_io(io::ErrorKind::NotFound));
            }
            Err(error) => return Err(error),
        };
        let file_type = metadata.file_type();
        if file_type.is_file() {
            if hops > 0 && metadata.uid() != owner {
                return Err(not_owned(false, current));
            }
            return Ok(Destination {
                path: current,
                mode: Some(metadata.mode() & 0o777),
            });
        }
        if !file_type.is_symlink() || policy == LinkPolicy::Reject {
            return Err(non_regular(&current));
        }
        if hops == MAX_LINK_HOPS {
            return Err(DestinationError::TooManyLinks {
                path: path.to_path_buf(),
            }
            .into_io(io::ErrorKind::InvalidInput));
        }
        if metadata.uid() != owner {
            return Err(not_owned(true, current));
        }
        let target = fs::read_link(&current)?;
        current = match current.parent() {
            Some(parent) if target.is_relative() => parent.join(target),
            _ => target,
        };
        hops += 1;
    }
}

fn validate_destination(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(non_regular(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn non_regular(path: &Path) -> io::Error {
    DestinationError::NonRegular {
        path: path.to_path_buf(),
    }
    .into_io(io::ErrorKind::InvalidInput)
}

fn not_owned(symlink: bool, path: PathBuf) -> io::Error {
    DestinationError::NotOwned { symlink, path }.into_io(io::ErrorKind::PermissionDenied)
}

fn create_temporary_file(parent: &Path) -> io::Result<(PathBuf, File)> {
    loop {
        let path = parent.join(format!(
            ".strata-write-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
