// SPDX-License-Identifier: MIT

use super::{
    deletion::local_device_is_rotational,
    fs_ops::{System, assert_off_main_thread},
    native_fs::opened_path,
};
use crate::adapters::volume::{MountSite, MountTable, MountType, mount_watch};
use gtk::{gio, glib, prelude::*};
use rustix::{
    fs::{AtFlags, StatxFlags},
    io::Errno,
};
use std::{os::fd::BorrowedFd, path::Path};

pub(super) trait FsEnvironment: Send + Sync {
    fn can_trash(&self, path: &Path) -> bool;
    /// GLib's own result: `Ok(false)` is a failure without an error (GLib #4060).
    fn trash(&self, path: &Path, cancellable: &gio::Cancellable) -> Result<bool, glib::Error>;
    fn mount_type(&self, fd: BorrowedFd<'_>) -> Option<(MountType, MountSite)>;
    /// udev's `ID_FS_TYPE`, when the device has a database record.
    fn device_format(&self, fd: BorrowedFd<'_>) -> Result<Option<String>, Errno>;
    fn rotational(&self, fd: BorrowedFd<'_>) -> Option<bool>;
}

impl FsEnvironment for System {
    fn can_trash(&self, path: &Path) -> bool {
        assert_off_main_thread();
        gio::File::for_path(path)
            .query_info(
                gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            )
            .is_ok_and(|info| info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH))
    }

    #[expect(
        unsafe_code,
        reason = "gio-rs asserts that FALSE always carries an error, which GLib #4060 breaks"
    )]
    fn trash(&self, path: &Path, cancellable: &gio::Cancellable) -> Result<bool, glib::Error> {
        use glib::translate::{ToGlibPtr, from_glib_full};

        assert_off_main_thread();
        let file = gio::File::for_path(path);
        let mut error = std::ptr::null_mut();
        // SAFETY: file and cancellable outlive the call, and error is a valid out-pointer.
        let trashed = unsafe {
            gio::ffi::g_file_trash(
                file.to_glib_none().0,
                cancellable.to_glib_none().0,
                &mut error,
            )
        };
        if error.is_null() {
            Ok(trashed != glib::ffi::GFALSE)
        } else {
            // SAFETY: GLib set the error and transferred its ownership to us.
            Err(unsafe { from_glib_full(error) })
        }
    }

    fn mount_type(&self, fd: BorrowedFd<'_>) -> Option<(MountType, MountSite)> {
        assert_off_main_thread();
        let owned =
            |(mount_type, site): (&MountType, &MountSite)| (mount_type.clone(), site.clone());
        match rustix::fs::statx(
            fd,
            "",
            AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC,
            StatxFlags::MNT_ID,
        ) {
            Ok(stat)
                if StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID) =>
            {
                let id = stat.stx_mnt_id;
                mount_watch::latest()
                    .mount_of(id)
                    .map(owned)
                    .or_else(|| MountTable::current().mount_of(id).map(owned))
            }
            // Before Linux 5.8.
            _ => MountTable::current()
                .mount_at(&opened_path(fd).ok()?)
                .map(owned),
        }
    }

    fn device_format(&self, fd: BorrowedFd<'_>) -> Result<Option<String>, Errno> {
        assert_off_main_thread();
        let stat = rustix::fs::statx(
            fd,
            "",
            AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC,
            StatxFlags::empty(),
        )?;
        let record = format!(
            "/run/udev/data/b{}:{}",
            stat.stx_dev_major, stat.stx_dev_minor
        );
        let record = match std::fs::read_to_string(record) {
            Ok(record) => record,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error
                    .raw_os_error()
                    .map_or(Errno::IO, Errno::from_raw_os_error));
            }
        };
        Ok(record
            .lines()
            .find_map(|line| line.strip_prefix("E:ID_FS_TYPE="))
            .map(str::to_owned))
    }

    fn rotational(&self, fd: BorrowedFd<'_>) -> Option<bool> {
        local_device_is_rotational(fd)
    }
}

#[cfg(test)]
mod tests;
