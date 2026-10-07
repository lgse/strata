// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    ffi::{OsStr, OsString},
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::extraction::MemberMetadata;

/// Reading procfs avoids a process-global umask(2) race in the multithreaded GUI.
pub(super) fn process_umask() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("Umask:")
                    .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
            })
        })
        .unwrap_or(0o022)
}

pub(super) fn permission_bits(mode: u32) -> u32 {
    mode & 0o777
}

fn timestamps(modified: SystemTime) -> Option<rustix::fs::Timestamps> {
    let since_epoch = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec {
            tv_sec: 0,
            tv_nsec: rustix::fs::UTIME_OMIT,
        },
        last_modification: rustix::fs::Timespec {
            tv_sec: i64::try_from(since_epoch.as_secs()).ok()?,
            tv_nsec: since_epoch.subsec_nanos().into(),
        },
    })
}

/// FAT and some network/FUSE mounts cannot store Unix permissions or timestamps.
fn best_effort(result: rustix::io::Result<()>) -> rustix::io::Result<()> {
    match result {
        Err(rustix::io::Errno::PERM | rustix::io::Errno::OPNOTSUPP | rustix::io::Errno::INVAL) => {
            Ok(())
        }
        result => result,
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MetadataCalls {
    pub(super) chmod: fn(BorrowedFd<'_>, rustix::fs::Mode) -> rustix::io::Result<()>,
    pub(super) set_times: fn(BorrowedFd<'_>, &rustix::fs::Timestamps) -> rustix::io::Result<()>,
    pub(super) set_link_times:
        fn(BorrowedFd<'_>, &OsStr, &rustix::fs::Timestamps) -> rustix::io::Result<()>,
}

impl MetadataCalls {
    pub(super) const SYSTEM: Self = Self {
        chmod: |directory, mode| rustix::fs::fchmod(directory, mode),
        set_times: |file, times| rustix::fs::futimens(file, times),
        set_link_times: |parent, name, times| {
            rustix::fs::utimensat(parent, name, times, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        },
    };
}

/// Clamp parent traversal at the extraction root so one such member does not abort the archive.
pub(super) fn sanitized_archive_path(name: impl AsRef<OsStr>) -> Result<PathBuf, String> {
    let native = name.as_ref();
    let name = native.to_string_lossy();
    let normalized: Vec<_> = native
        .as_bytes()
        .iter()
        .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
        .collect();
    if normalized.is_empty() || normalized.starts_with(b"/") {
        return Err(format!("Refusing unsafe archive path: {name}"));
    }

    let mut path = PathBuf::new();
    for component in normalized.split(|byte| *byte == b'/') {
        match component {
            b"" | b"." => {}
            b".." => {
                path.pop();
            }
            bytes if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' => {
                return Err(format!("Refusing unsafe archive path: {name}"));
            }
            _ => path.push(OsStr::from_bytes(component)),
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

/// Pinned roots and NOFOLLOW traversal prevent symlink swaps from redirecting writes.
#[derive(Debug)]
pub(super) struct ExtractionDestination {
    root: OwnedFd,
    calls: MetadataCalls,
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
        Ok(Self {
            root,
            calls: MetadataCalls::SYSTEM,
        })
    }

    #[cfg(test)]
    pub(super) fn with_metadata_calls(self, calls: MetadataCalls) -> Self {
        Self { calls, ..self }
    }

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
            let calls = self.calls;
            return Ok((OsString::from(name), Self { root, calls }));
        }
        Err("Could not create the extraction staging folder: no unused name".to_owned())
    }

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

    pub(super) fn remove_directory_only_staging(&self, staging: &OsStr) -> Result<bool, String> {
        remove_tree(self.root.as_fd(), staging, false)
            .map_err(|error| format!("Could not remove the extraction staging folder: {error}"))
    }

    /// Never follows symlinks. Removal errors can leave a partially deleted tree.
    pub(super) fn remove_staging(&self, staging: &OsStr) -> Result<(), String> {
        remove_tree(self.root.as_fd(), staging, true)
            .map(|_| ())
            .map_err(|error| format!("Could not remove the extraction staging folder: {error}"))
    }

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
                    rustix::fs::FileType::RegularFile
                    | rustix::fs::FileType::Directory
                    | rustix::fs::FileType::Symlink => {}
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
        self.descend(path, true)
    }

    pub(super) fn open_directory(&self, path: &Path) -> Result<OwnedFd, String> {
        self.descend(path, false)
    }

    fn descend(&self, path: &Path, create: bool) -> Result<OwnedFd, String> {
        let mut directory = self.root.try_clone().map_err(|error| error.to_string())?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                return Err("Invalid internal extraction path".to_owned());
            };
            if create {
                match rustix::fs::mkdirat(&directory, name, rustix::fs::Mode::from_raw_mode(0o777))
                {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(error.to_string()),
                }
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

    fn prepare_leaf(&self, path: &Path) -> Result<(OwnedFd, OsString, PathBuf), String> {
        let parent = self.create_directories(path.parent().unwrap_or_else(|| Path::new("")))?;
        let name = path
            .file_name()
            .ok_or_else(|| "Archive entry has no file name".to_owned())?;
        let name = self.available_name(&parent, name)?;
        let created = path.with_file_name(&name);
        Ok((parent, name, created))
    }

    pub(super) fn create_file(
        &self,
        path: &Path,
        mode: Option<u32>,
    ) -> Result<(std::fs::File, PathBuf), String> {
        let (parent, name, created) = self.prepare_leaf(path)?;
        let file = rustix::fs::openat(
            parent,
            name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(mode.map_or(0o666, permission_bits)),
        )
        .map(std::fs::File::from)
        .map_err(|error| error.to_string())?;
        Ok((file, created))
    }

    pub(super) fn create_symlink(
        &self,
        path: &Path,
        target: &OsStr,
        modified: Option<SystemTime>,
    ) -> Result<PathBuf, String> {
        let (parent, name, created) = self.prepare_leaf(path)?;
        rustix::fs::symlinkat(target, &parent, &name).map_err(|error| {
            format!(
                "Could not create symbolic link `{}`: {error}",
                created.display()
            )
        })?;
        if let Some(times) = modified.and_then(timestamps)
            && let Err(error) =
                best_effort((self.calls.set_link_times)(parent.as_fd(), &name, &times))
        {
            let _ = rustix::fs::unlinkat(&parent, &name, rustix::fs::AtFlags::empty());
            return Err(format!(
                "Could not restore the modification time of `{}`: {error}",
                created.display()
            ));
        }
        Ok(created)
    }

    pub(super) fn set_file_times(
        &self,
        file: &std::fs::File,
        modified: SystemTime,
    ) -> rustix::io::Result<()> {
        match timestamps(modified) {
            Some(times) => best_effort((self.calls.set_times)(file.as_fd(), &times)),
            None => Ok(()),
        }
    }

    /// Link symlinks themselves rather than following their targets outside staging.
    pub(super) fn create_hard_link(&self, path: &Path, target: &Path) -> Result<PathBuf, String> {
        let unavailable = |error: String| {
            format!(
                "Hard link `{}` refers to `{}`, which is not available in this extraction: {error}",
                path.display(),
                target.display()
            )
        };
        let target_name = target
            .file_name()
            .ok_or_else(|| unavailable("no file name".to_owned()))?;
        let target_parent = self
            .open_directory(target.parent().unwrap_or_else(|| Path::new("")))
            .map_err(unavailable)?;
        let (parent, name, created) = self.prepare_leaf(path)?;
        rustix::fs::linkat(
            &target_parent,
            target_name,
            &parent,
            &name,
            rustix::fs::AtFlags::empty(),
        )
        .map_err(|error| {
            format!(
                "Could not link `{}` to `{}`: {error}",
                created.display(),
                target.display()
            )
        })?;
        Ok(created)
    }

    pub(super) fn apply_directory_metadata(
        &self,
        path: &Path,
        metadata: MemberMetadata,
        umask: u32,
    ) -> Result<(), String> {
        let directory = self
            .open_directory(path)
            .map_err(|error| format!("Could not open `{}`: {error}", path.display()))?;
        if let Some(mode) = metadata.mode {
            let mode = rustix::fs::Mode::from_raw_mode(permission_bits(mode) & !umask);
            best_effort((self.calls.chmod)(directory.as_fd(), mode)).map_err(|error| {
                format!(
                    "Could not restore permissions on `{}`: {error}",
                    path.display()
                )
            })?;
        }
        if let Some(times) = metadata.modified.and_then(timestamps) {
            best_effort((self.calls.set_times)(directory.as_fd(), &times)).map_err(|error| {
                format!(
                    "Could not restore the modification time of `{}`: {error}",
                    path.display()
                )
            })?;
        }
        Ok(())
    }

    /// Keep failed output removable; the original metadata error is already reported.
    pub(super) fn reset_directory_mode(&self, path: &Path, umask: u32) {
        if let Ok(directory) = self.open_directory(path) {
            let mode = rustix::fs::Mode::from_raw_mode(0o777 & !umask);
            let _ = (self.calls.chmod)(directory.as_fd(), mode);
        }
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

/// Without RENAME_NOREPLACE, only non-directories have a safe link/unlink fallback.
fn move_without_replacing(
    from_directory: BorrowedFd<'_>,
    from: &OsStr,
    to_directory: BorrowedFd<'_>,
    to: &OsStr,
    rename: RenameNoReplace,
) -> rustix::io::Result<()> {
    match rename(from_directory, from, to_directory, to) {
        Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOSYS | rustix::io::Errno::OPNOTSUPP) => {
            move_by_link(from_directory, from, to_directory, to)
        }
        result => result,
    }
}

fn move_by_link(
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
    // A mkdir reservation can be replaced before renameat, which would clobber it.
    Err(rustix::io::Errno::OPNOTSUPP)
}

/// Never follows symlinks. With `files` disabled, any non-directory preserves its ancestors.
fn remove_tree(parent: BorrowedFd<'_>, name: &OsStr, files: bool) -> rustix::io::Result<bool> {
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
        let is_directory = file_type == rustix::fs::FileType::Directory;
        if !is_directory && !files {
            return Ok(false);
        }
        children.push((child.to_os_string(), is_directory));
    }
    for (child, is_directory) in children {
        if !is_directory {
            rustix::fs::unlinkat(&directory, &child, rustix::fs::AtFlags::empty())?;
        } else if !remove_tree(directory.as_fd(), &child, files)? {
            return Ok(false);
        }
    }
    rustix::fs::unlinkat(parent, name, rustix::fs::AtFlags::REMOVEDIR)?;
    Ok(true)
}

/// Nested members must share their root's conflict rename.
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
