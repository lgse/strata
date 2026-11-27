// SPDX-License-Identifier: MIT

use std::{
    ffi::{CStr, OsStr},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use glib::translate::ToGlibPtr;
use rustix::{
    fs::{self, Access, Mode, OFlags, ResolveFlags},
    io::Errno,
};

use super::trash_support::sticky_allows;
use crate::{model::Location, services::DeleteBlocker};

const VERIFY_BUDGET: Duration = Duration::from_secs(2);

impl DeleteBlocker {
    /// Resolves the blocker without following symlinks from the deleted folder down.
    /// The final entry can itself be a symlink: revealing it does not open it.
    pub async fn reveal_target(&self) -> Option<(Location, Location)> {
        let blocker = self.clone();
        let path = verify(VERIFY_BUDGET, move || {
            Some(blocker.root.join(blocker.resolve()?))
        })
        .await?;
        Some((Location::local(path.parent()?), Location::local(path)))
    }

    /// The first blocker that can be revealed, as what would be shown for it.
    pub(super) async fn first_revealable(blockers: Vec<Self>) -> Option<Self> {
        if blockers.is_empty() {
            return None;
        }
        verify(VERIFY_BUDGET, move || {
            blockers.into_iter().find_map(|blocker| {
                let child = blocker.resolve()?;
                Some(Self { child, ..blocker })
            })
        })
        .await
    }

    /// The blocker, or the folder it is in where that folder cannot be entered.
    fn resolve(&self) -> Option<PathBuf> {
        // The delete follows links above its target too.
        let parent = fs::openat2(
            fs::CWD,
            self.root.parent()?,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_MAGICLINKS,
        )
        .ok()?;
        let root = Path::new(self.root.file_name()?);
        for child in self.child.ancestors() {
            if child.as_os_str().is_empty() {
                break;
            }
            match fs::openat2(
                &parent,
                root.join(child),
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            ) {
                Ok(_) => return Some(child.to_owned()),
                Err(Errno::ACCESS) => {}
                Err(_) => return None,
            }
        }
        None
    }
}

/// Gives up on a mount that stops answering, so the failure is still reported.
async fn verify<T: Send + 'static>(
    budget: Duration,
    work: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    glib::future_with_timeout(budget, gio::spawn_blocking(work))
        .await
        .ok()?
        .ok()
        .flatten()
}

/// Local GIO exposes the refused child only in its error text. Recognize its
/// specific diagnostics, not arbitrary paths mentioned by a Trash backend.
/// Blocking: checks what the reported entry itself allows.
pub(super) fn reported_trash_child(root: &Path, error: &glib::Error) -> Option<PathBuf> {
    if !error.matches(gio::IOErrorEnum::PermissionDenied) {
        return None;
    }
    let message = error_bytes(error);
    let format = glib::dgettext(Some("glib20"), "Unable to trash child file %s");
    if let Some(path) = interpolate_path(message, format.as_bytes()) {
        let child = child_of(root, Path::new(OsStr::from_bytes(path)))?;
        return refusing_entry(root, child);
    }
    child_of(root, &unreadable_directory(message)?)
}

fn child_of(root: &Path, path: &Path) -> Option<PathBuf> {
    let child = path.strip_prefix(root).ok()?;
    let mut parts = child.components();
    if !root.is_absolute() || !matches!(parts.next(), Some(Component::Normal(_))) {
        return None;
    }
    parts
        .all(|part| matches!(part, Component::Normal(_)))
        .then(|| child.to_owned())
}

/// GIO refuses a folder the user does not own by naming the first entry it lists
/// there, so the folder is the blocker unless that entry is blocked as well.
/// `None` when that folder is the requested target itself.
fn refusing_entry(root: &Path, child: PathBuf) -> Option<PathBuf> {
    if removal_refused(&root.join(&child)) {
        return Some(child);
    }
    let folder = child.parent()?;
    (!folder.as_os_str().is_empty()).then(|| folder.to_owned())
}

fn removal_refused(entry: &Path) -> bool {
    let Some(folder) = entry.parent() else {
        return false;
    };
    let (Ok(in_folder), Ok(stat)) = (folder.symlink_metadata(), entry.symlink_metadata()) else {
        return false;
    };
    fs::access(folder, Access::WRITE_OK | Access::EXEC_OK).is_err()
        || !sticky_allows(&in_folder, &stat)
}

fn interpolate_path<'a>(message: &'a [u8], format: &[u8]) -> Option<&'a [u8]> {
    for placeholder in [b"%s".as_slice(), b"%1$s"] {
        if let Some(pos) = format
            .windows(placeholder.len())
            .position(|part| part == placeholder)
        {
            let (before, rest) = format.split_at(pos);
            return message
                .strip_prefix(before)?
                .strip_suffix(&rest[placeholder.len()..]);
        }
    }
    None
}

fn unreadable_directory(message: &[u8]) -> Option<PathBuf> {
    // GLocalFileEnumerator emits this unlocalized format, with a UTF-8 filename.
    let detail = std::str::from_utf8(message)
        .ok()?
        .strip_prefix("Error opening directory '")?;
    let (path, _) = detail.rsplit_once("': ")?;
    glib::filename_from_utf8(path).ok().map(|(path, _)| path)
}

#[expect(
    unsafe_code,
    reason = "glib::Error::message truncates a GIO path at the first non-UTF-8 byte"
)]
fn error_bytes(error: &glib::Error) -> &[u8] {
    let ptr: *const glib::ffi::GError = error.to_glib_none().0;
    // SAFETY: `ptr` is the live GError borrowed from `error`.
    let message = unsafe { (*ptr).message };
    // SAFETY: the GError owns its NUL-terminated message for the same lifetime.
    unsafe { CStr::from_ptr(message) }.to_bytes()
}

#[cfg(test)]
pub(super) mod tests;
