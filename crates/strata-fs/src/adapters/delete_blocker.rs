// SPDX-License-Identifier: MIT

use std::{
    ffi::{CStr, OsStr},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

use glib::translate::ToGlibPtr;
use rustix::fs::{self, Mode, OFlags, ResolveFlags};

use crate::{model::Location, services::DeleteBlocker};

impl DeleteBlocker {
    /// Resolves the blocker without following symlinks on the way to its folder.
    /// The final entry can itself be a symlink: revealing it does not open it.
    pub async fn reveal_target(&self) -> Option<(Location, Location)> {
        let blocker = self.clone();
        gio::spawn_blocking(move || blocker.resolve())
            .await
            .ok()
            .flatten()
    }

    fn resolve(&self) -> Option<(Location, Location)> {
        let no_links = ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS;
        let root = fs::openat2(
            fs::CWD,
            &self.root,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            no_links,
        )
        .ok()?;
        fs::openat2(
            &root,
            &self.child,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            no_links | ResolveFlags::BENEATH,
        )
        .ok()?;
        let path = self.root.join(&self.child);
        Some((Location::local(path.parent()?), Location::local(path)))
    }
}

/// Local GIO exposes the refused child only in its error text. Recognize its
/// specific diagnostics, not arbitrary paths mentioned by a Trash backend.
pub(super) fn reported_trash_child(root: &Path, error: &glib::Error) -> Option<PathBuf> {
    if !error.matches(gio::IOErrorEnum::PermissionDenied) {
        return None;
    }
    let message = error_bytes(error);
    let format = glib::dgettext(Some("glib20"), "Unable to trash child file %s");
    let path = if let Some(path) = interpolate_path(message, format.as_bytes()) {
        PathBuf::from(OsStr::from_bytes(path))
    } else {
        unreadable_directory(message)?
    };
    let child = path.strip_prefix(root).ok()?;
    let mut parts = child.components();
    if !root.is_absolute() || !matches!(parts.next(), Some(Component::Normal(_))) {
        return None;
    }
    parts
        .all(|part| matches!(part, Component::Normal(_)))
        .then(|| child.to_owned())
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
mod tests;
