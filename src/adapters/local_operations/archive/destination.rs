// SPDX-License-Identifier: MIT

//! Confined destination writes and extraction conflict naming.

#[cfg(test)]
mod tests;

use std::{
    ffi::{OsStr, OsString},
    os::{
        fd::{AsFd, OwnedFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Component, Path, PathBuf},
};

/// Converts an archive member name into a relative path that cannot escape the destination.
///
/// Normalizes backslashes to slashes, skips empty and `.` components, and
/// rejects absolute paths, `..`, Windows drive prefixes (`C:`), and names
/// that collapse to empty.
///
/// # Errors
///
/// Returns an error if `name` is empty, absolute, contains `..`, includes a
/// drive prefix, or has no remaining components after normalization.
pub(super) fn validated_archive_path(name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return Err(format!("Refusing unsafe archive path: {name}"));
    }

    let mut path = PathBuf::new();
    for component in normalized.split('/') {
        match component.as_bytes() {
            b"" | b"." => {}
            b".." => return Err(format!("Refusing unsafe archive path: {name}")),
            bytes if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' => {
                return Err(format!("Refusing unsafe archive path: {name}"));
            }
            _ => path.push(component),
        }
    }
    if path.as_os_str().is_empty() {
        return Err(format!("Refusing empty archive path: {name}"));
    }
    Ok(path)
}

/// Returns `name` with ` ({index})` inserted before the extension.
///
/// Used by [`ExtractionDestination::available_name`] to pick `readme (2).txt`
/// when `readme.txt` already exists.
fn suffixed_name(name: &OsStr, index: u64) -> OsString {
    let path = Path::new(name);
    let mut candidate = path.file_stem().unwrap_or(name).as_bytes().to_vec();
    candidate.extend_from_slice(format!(" ({index})").as_bytes());
    if let Some(extension) = path.extension() {
        candidate.push(b'.');
        candidate.extend_from_slice(extension.as_bytes());
    }
    OsString::from_vec(candidate)
}

/// Pinned destination directory for extraction.
///
/// All member creates go through this root with `NOFOLLOW`, so a symlink
/// swapped into the destination tree cannot redirect writes outside it.
pub(super) struct ExtractionDestination {
    root: OwnedFd,
}

impl ExtractionDestination {
    /// Opens `path` as a directory. Ordinary symlinks along the path are
    /// resolved, since users browse symlinked folders, but the resolved
    /// directory is pinned so later replacement of any component cannot
    /// redirect writes. This mirrors how copy and compress open their parents.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is not absolute or cannot be opened as a directory.
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        let relative = path
            .strip_prefix("/")
            .map_err(|_| "Extraction destination must use an absolute path".to_owned())?;
        let filesystem_root = rustix::fs::open(
            c"/",
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("Could not open the filesystem root: {error}"))?;
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        let root = rustix::fs::openat2(
            &filesystem_root,
            relative,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::IN_ROOT | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|error| format!("Could not open extraction destination: {error}"))?;
        Ok(Self { root })
    }

    /// Uses [`fstatvfs`] on the pinned root so a swapped path cannot redirect
    /// the query. A zero `f_blocks` means the filesystem does not report
    /// capacity, so callers skip the check instead of refusing every extraction.
    ///
    /// [`fstatvfs`]: rustix::fs::fstatvfs
    pub(super) fn available_bytes(&self) -> Result<Option<u64>, String> {
        let stat = rustix::fs::fstatvfs(&self.root).map_err(|error| {
            format!("Could not inspect free space at the extraction destination: {error}")
        })?;
        if stat.f_blocks == 0 {
            return Ok(None);
        }
        let block = if stat.f_frsize > 0 {
            stat.f_frsize
        } else {
            stat.f_bsize.max(1)
        };
        Ok(Some(stat.f_bavail.saturating_mul(block)))
    }

    /// Finds a name in `directory` that does not already exist.
    ///
    /// Tries `name`, then [`suffixed_name`] with increasing indexes. Existing
    /// regular files and directories are skipped; special filesystem objects
    /// (devices, sockets, existing symlinks) are refused rather than overwritten.
    ///
    /// # Errors
    ///
    /// Returns an error if `directory` cannot be inspected or an existing
    /// candidate is a special filesystem object.
    fn available_name<Fd: AsFd>(&self, directory: &Fd, name: &OsStr) -> Result<OsString, String> {
        for index in 1.. {
            let candidate = if index == 1 {
                name.to_owned()
            } else {
                suffixed_name(name, index)
            };
            match rustix::fs::statat(directory, &candidate, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => return Ok(candidate),
                Err(error) => {
                    return Err(format!(
                        "Could not inspect extraction path {}: {error}",
                        candidate.to_string_lossy()
                    ));
                }
                Ok(stat) => match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
                    rustix::fs::FileType::RegularFile | rustix::fs::FileType::Directory => {}
                    _ => {
                        return Err(format!(
                            "Refusing to extract over special filesystem object: {}",
                            candidate.to_string_lossy()
                        ));
                    }
                },
            }
        }
        Err(format!(
            "Could not find an available extraction name for {}",
            name.to_string_lossy()
        ))
    }

    /// Creates each component of `path` under the destination root and returns the leaf directory.
    ///
    /// Existing directories are reused. Each component is opened with
    /// [`DIRECTORY`] and [`NOFOLLOW`], so a symlink cannot be followed as a
    /// directory.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` contains a non-normal component, a component
    /// cannot be created, or a component exists but is not a directory.
    ///
    /// [`DIRECTORY`]: rustix::fs::OFlags::DIRECTORY
    /// [`NOFOLLOW`]: rustix::fs::OFlags::NOFOLLOW
    pub(super) fn create_directories(&self, path: &Path) -> Result<OwnedFd, String> {
        let mut directory = self.root.try_clone().map_err(|error| error.to_string())?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                return Err("Invalid internal extraction path".to_owned());
            };
            match rustix::fs::mkdirat(&directory, name, rustix::fs::Mode::from_raw_mode(0o777)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.to_string()),
            }
            directory = rustix::fs::openat(
                &directory,
                name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(directory)
    }

    /// Creates the file at `path`, renaming the leaf if that name is already taken.
    ///
    /// Parent directories are created with [`Self::create_directories`]. The
    /// leaf is opened with [`CREATE`], [`EXCL`], and [`NOFOLLOW`] so an existing
    /// file or symlink is never overwritten. Returns the open file and the
    /// relative path actually created, which may differ from `path` after a
    /// rename.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` has no file name, a parent cannot be created,
    /// no unused name can be found, or the exclusive create fails.
    ///
    /// [`CREATE`]: rustix::fs::OFlags::CREATE
    /// [`EXCL`]: rustix::fs::OFlags::EXCL
    /// [`NOFOLLOW`]: rustix::fs::OFlags::NOFOLLOW
    pub(super) fn create_file(&self, path: &Path) -> Result<(std::fs::File, PathBuf), String> {
        let parent = self.create_directories(path.parent().unwrap_or_else(|| Path::new("")))?;
        let name = path
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        let name = self.available_name(&parent, name)?;
        let mut created = PathBuf::new();
        if let Some(parent_path) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            created.push(parent_path);
        }
        created.push(&name);
        let file = rustix::fs::openat(
            parent,
            name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o666),
        )
        .map(std::fs::File::from)
        .map_err(|error| error.to_string())?;
        Ok((file, created))
    }

    /// Unlinks the leaf of `path` under the destination root.
    ///
    /// Used to discard a partially written member after cancellation or
    /// copy failure. Does not follow a final symbolic link.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` has no file name, a parent cannot be opened,
    /// or the unlink fails.
    pub(super) fn remove_file(&self, path: &Path) -> Result<(), String> {
        let parent = self.create_directories(path.parent().unwrap_or_else(|| Path::new("")))?;
        let name = path
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        rustix::fs::unlinkat(&parent, name, rustix::fs::AtFlags::empty()).map_err(|error| {
            format!(
                "Could not remove incomplete extraction {}: {error}",
                path.display()
            )
        })
    }
}

/// Tracks renamed top-level entries so nested members follow the same rename.
///
/// If `docs` already exists in the destination, a member `docs/readme.txt`
/// is extracted under `docs (2)/readme.txt` rather than merging into `docs`.
pub(super) struct ExtractNameResolver {
    renames: std::collections::HashMap<OsString, OsString>,
}

impl ExtractNameResolver {
    pub(super) fn new() -> Self {
        Self {
            renames: std::collections::HashMap::new(),
        }
    }

    /// Maps a validated relative member path onto a conflict-free destination path.
    ///
    /// The top-level component is passed through [`ExtractionDestination::available_name`]
    /// once and remembered for later members that share that prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` has no normal first component or
    /// [`ExtractionDestination::available_name`] fails.
    pub(super) fn resolve(
        &mut self,
        destination: &ExtractionDestination,
        path: &Path,
    ) -> Result<PathBuf, String> {
        let top = path
            .components()
            .next()
            .and_then(|component| match component {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        if !self.renames.contains_key(top) {
            let name = destination.available_name(&destination.root, top)?;
            self.renames.insert(top.to_owned(), name);
        }
        Ok(self.apply_known_rename(path))
    }

    /// Maps a validated path without filesystem probes or reserving a new name.
    pub(super) fn apply_known_rename(&self, path: &Path) -> PathBuf {
        let mut components = path.iter();
        let Some(top) = components.next() else {
            return path.to_path_buf();
        };
        let top = self
            .renames
            .get(top)
            .map(OsString::as_os_str)
            .unwrap_or(top);
        let mut resolved = PathBuf::from(top);
        resolved.extend(components);
        resolved
    }
}
