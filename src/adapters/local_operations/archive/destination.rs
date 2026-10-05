// SPDX-License-Identifier: MIT

//! Confined destination writes, the extraction staging folder lifecycle,
//! publication and conflict naming.

#[cfg(test)]
mod tests;

use std::{
    ffi::{OsStr, OsString},
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Component, Path, PathBuf},
};

/// Clamp parent traversal at the extraction root so one such member does not abort the archive.
pub(super) fn sanitized_archive_path(name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return Err(format!("Refusing unsafe archive path: {name}"));
    }

    let mut path = PathBuf::new();
    for component in normalized.split('/') {
        match component.as_bytes() {
            b"" | b"." => {}
            b".." => {
                path.pop();
            }
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

/// Archive name without a known archive extension, used to name the folder
/// that holds several extracted entries. Falls back to the full name when
/// nothing would remain, such as for an archive called `.zip`.
pub(super) fn archive_stem(archive_name: &str) -> &str {
    let lower = archive_name.to_ascii_lowercase();
    let stem = [".tar.gz", ".tgz", ".tar", ".zip", ".7z", ".rar"]
        .iter()
        .find_map(|suffix| {
            lower
                .ends_with(suffix)
                .then(|| &archive_name[..archive_name.len() - suffix.len()])
        })
        .unwrap_or(archive_name);
    if stem.trim_matches('.').is_empty() {
        archive_name
    } else {
        stem
    }
}

/// Pinned directory for extraction.
///
/// One instance pins the user's destination and a second one pins the hidden
/// staging folder created under it, which receives every member. All member
/// creates go through the staging root with `NOFOLLOW`, so a symlink swapped
/// into the destination tree cannot redirect writes outside it.
#[derive(Debug)]
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

    /// Creates an empty hidden `.strata-extraction-<uuid>` folder under the
    /// pinned root and pins it as a second destination. Members are written
    /// there and published by name once the outcome is known.
    ///
    /// # Errors
    ///
    /// Returns an error if no fresh name can be created or the folder cannot
    /// be opened without following a symlink.
    pub(super) fn create_staging(&self) -> Result<(OsString, Self), String> {
        for _ in 0..8 {
            let name = format!(".strata-extraction-{}", gtk::glib::uuid_string_random());
            match rustix::fs::mkdirat(&self.root, &name, rustix::fs::Mode::from_raw_mode(0o777)) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => {
                    return Err(format!(
                        "Could not create the extraction staging folder: {error}"
                    ));
                }
            }
            let root = rustix::fs::openat(
                &self.root,
                &name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|error| {
                let _ = rustix::fs::unlinkat(&self.root, &name, rustix::fs::AtFlags::REMOVEDIR);
                format!("Could not open the extraction staging folder: {error}")
            })?;
            return Ok((OsString::from(name), Self { root }));
        }
        Err("Could not create the extraction staging folder: no unused name".to_owned())
    }

    /// Removes `staging` if it is empty. Returns `Ok(false)` when it still
    /// holds entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the folder cannot be removed for another reason.
    pub(super) fn remove_empty_staging(&self, staging: &OsStr) -> Result<bool, String> {
        match rustix::fs::unlinkat(&self.root, staging, rustix::fs::AtFlags::REMOVEDIR) {
            Ok(()) => Ok(true),
            // Some filesystems report a non-empty directory as EEXIST.
            Err(rustix::io::Errno::NOTEMPTY | rustix::io::Errno::EXIST) => Ok(false),
            Err(error) => Err(format!(
                "Could not remove the extraction staging folder: {error}"
            )),
        }
    }

    /// Removes `staging` when it holds nothing but directories, deepest first.
    /// Returns `Ok(false)`, leaving everything in place, as soon as a file or
    /// other non-directory entry is found.
    ///
    /// # Errors
    ///
    /// Returns an error if a directory cannot be read or removed.
    pub(super) fn remove_directory_only_staging(&self, staging: &OsStr) -> Result<bool, String> {
        remove_directory_only_tree(self.root.as_fd(), staging)
            .map_err(|error| format!("Could not remove the extraction staging folder: {error}"))
    }

    /// Renames `staging` to the archive stem, or `stem (n)` when that name is
    /// taken, without replacing anything. Returns the published name.
    ///
    /// # Errors
    ///
    /// Returns an error if the rename fails for a reason other than a taken
    /// name; `staging` is then left in place.
    pub(super) fn publish_staging_as_folder(
        &self,
        staging: &OsStr,
        archive_name: &str,
    ) -> Result<String, String> {
        self.publish_staging_as_folder_with(staging, archive_name, rename_no_replace)
    }

    fn publish_staging_as_folder_with(
        &self,
        staging: &OsStr,
        archive_name: &str,
        rename: RenameNoReplace,
    ) -> Result<String, String> {
        let stem = archive_stem(archive_name);
        if stem.contains('/') {
            return Err(format!("Invalid extraction folder name `{stem}`"));
        }
        for suffix in 0_u64.. {
            let name = if suffix == 0 {
                stem.to_owned()
            } else {
                format!("{stem} ({suffix})")
            };
            match move_without_replacing(
                self.root.as_fd(),
                staging,
                self.root.as_fd(),
                OsStr::new(&name),
                rename,
            ) {
                Ok(()) => return Ok(name),
                Err(
                    rustix::io::Errno::EXIST
                    | rustix::io::Errno::NOTEMPTY
                    | rustix::io::Errno::ISDIR
                    | rustix::io::Errno::NOTDIR,
                ) => {}
                Err(error) => {
                    return Err(format!(
                        "Could not publish the extracted entries as `{name}`: {error}"
                    ));
                }
            }
        }
        Err("No available extraction folder name".to_owned())
    }

    /// Moves the single top-level `root` out of `staging` into this
    /// destination under its own name, or `name (n)` when something already
    /// uses it. Existing entries, including symlinks and special files, are
    /// never replaced or followed. Returns the published name.
    ///
    /// # Errors
    ///
    /// Returns an error if the rename fails for a reason other than a taken
    /// name; `root` is then left in `staging`.
    pub(super) fn publish_single_root(
        &self,
        staging: &Self,
        root: &Path,
    ) -> Result<OsString, String> {
        self.publish_single_root_with(staging, root, rename_no_replace)
    }

    fn publish_single_root_with(
        &self,
        staging: &Self,
        root: &Path,
        rename: RenameNoReplace,
    ) -> Result<OsString, String> {
        let leaf = root
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        for index in 1.. {
            let candidate = if index == 1 {
                leaf.to_owned()
            } else {
                suffixed_name(leaf, index)
            };
            match move_without_replacing(
                staging.root.as_fd(),
                root.as_os_str(),
                self.root.as_fd(),
                &candidate,
                rename,
            ) {
                Ok(()) => return Ok(candidate),
                Err(
                    rustix::io::Errno::EXIST
                    | rustix::io::Errno::NOTEMPTY
                    | rustix::io::Errno::ISDIR
                    | rustix::io::Errno::NOTDIR,
                ) => {}
                Err(error) => {
                    return Err(format!(
                        "Could not publish `{}`: {error}",
                        candidate.to_string_lossy()
                    ));
                }
            }
        }
        Err(format!(
            "Could not find an available extraction name for {}",
            leaf.to_string_lossy()
        ))
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

/// `renameat2(RENAME_NOREPLACE)` between two pinned directories.
type RenameNoReplace = fn(BorrowedFd<'_>, &OsStr, BorrowedFd<'_>, &OsStr) -> rustix::io::Result<()>;

fn rename_no_replace(
    from_directory: BorrowedFd<'_>,
    from: &OsStr,
    to_directory: BorrowedFd<'_>,
    to: &OsStr,
) -> rustix::io::Result<()> {
    rustix::fs::renameat_with(
        from_directory,
        from,
        to_directory,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    )
}

/// Moves `from` to `to` without ever replacing an existing entry.
///
/// Filesystems without `RENAME_NOREPLACE` (some NFS and FUSE mounts) get a
/// fallback that keeps the no-clobber guarantee: a file is hard-linked (which
/// fails with `EEXIST`) and then unlinked; a directory first reserves `to` with
/// `mkdirat` and then replaces only that empty reservation. A plain rename is
/// never aimed at a name this function did not just create.
fn move_without_replacing(
    from_directory: BorrowedFd<'_>,
    from: &OsStr,
    to_directory: BorrowedFd<'_>,
    to: &OsStr,
    rename: RenameNoReplace,
) -> rustix::io::Result<()> {
    match rename(from_directory, from, to_directory, to) {
        Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOSYS | rustix::io::Errno::OPNOTSUPP) => {
            move_by_reservation(from_directory, from, to_directory, to)
        }
        result => result,
    }
}

fn move_by_reservation(
    from_directory: BorrowedFd<'_>,
    from: &OsStr,
    to_directory: BorrowedFd<'_>,
    to: &OsStr,
) -> rustix::io::Result<()> {
    let stat = rustix::fs::statat(from_directory, from, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Directory {
        rustix::fs::linkat(
            from_directory,
            from,
            to_directory,
            to,
            rustix::fs::AtFlags::empty(),
        )?;
        // A leftover source link is reported later as unpublished staging content.
        let _ = rustix::fs::unlinkat(from_directory, from, rustix::fs::AtFlags::empty());
        return Ok(());
    }
    rustix::fs::mkdirat(to_directory, to, rustix::fs::Mode::from_raw_mode(0o700))?;
    rustix::fs::renameat(from_directory, from, to_directory, to).inspect_err(|_| {
        // Only succeeds while the reservation is still an empty directory.
        let _ = rustix::fs::unlinkat(to_directory, to, rustix::fs::AtFlags::REMOVEDIR);
    })
}

fn remove_directory_only_tree(parent: BorrowedFd<'_>, name: &OsStr) -> rustix::io::Result<bool> {
    let directory = rustix::fs::openat(
        parent,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let mut children = Vec::new();
    for entry in rustix::fs::Dir::read_from(&directory)? {
        let entry = entry?;
        let child = OsStr::from_bytes(entry.file_name().to_bytes());
        if child == "." || child == ".." {
            continue;
        }
        let file_type = match entry.file_type() {
            rustix::fs::FileType::Unknown => rustix::fs::FileType::from_raw_mode(
                rustix::fs::statat(&directory, child, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)?
                    .st_mode,
            ),
            file_type => file_type,
        };
        if file_type != rustix::fs::FileType::Directory {
            return Ok(false);
        }
        children.push(child.to_os_string());
    }
    for child in children {
        if !remove_directory_only_tree(directory.as_fd(), &child)? {
            return Ok(false);
        }
    }
    rustix::fs::unlinkat(parent, name, rustix::fs::AtFlags::REMOVEDIR)?;
    Ok(true)
}

/// Tracks renamed top-level entries so nested members follow the same rename.
///
/// Extraction resolves names inside a fresh staging folder, so the leaf
/// conflict naming in [`ExtractionDestination::create_file`] only renames
/// duplicates within the archive (`same.txt`, then `same (2).txt`). A top-level
/// name is resolved once and reused for every later member under it.
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
