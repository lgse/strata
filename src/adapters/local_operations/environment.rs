// SPDX-License-Identifier: MIT

use super::{
    deletion::local_device_is_rotational,
    fs_ops::{Entry, EntryStat, System, Watched, assert_off_main_thread},
    holders,
    native_fs::{self, opened_path},
};
use crate::{
    adapters::volume::{MountSite, MountTable, MountType, mount_watch},
    services::Holder,
};
use gtk::{gio, glib, prelude::*};
use rustix::{
    fs::{AtFlags, Mode, OFlags, StatxFlags},
    io::Errno,
};
use std::{
    ffi::OsString,
    os::fd::{AsFd, BorrowedFd},
    path::Path,
};

pub(super) trait FsEnvironment: Send + Sync {
    fn can_trash(&self, path: &Path) -> bool;
    /// GLib's own result: `Ok(false)` is a failure without an error (GLib #4060).
    fn trash(&self, path: &Path, cancellable: &gio::Cancellable) -> Result<bool, glib::Error>;
    fn mount_type(&self, fd: BorrowedFd<'_>) -> Option<(MountType, MountSite)>;
    fn folder_mount(&self, path: &Path, id: Option<u64>) -> Option<(u64, MountType)>;
    fn watch(
        &self,
        file: &gio::File,
        watched: Watched,
        flags: gio::FileMonitorFlags,
    ) -> Result<gio::FileMonitor, glib::Error>;
    fn unwatch(&self, file: &gio::File, monitor: gio::FileMonitor);
    fn current_mount_type(&self, fd: BorrowedFd<'_>) -> Option<MountType>;
    fn file_handle_inputs(
        &self,
        at: Entry<'_>,
        stat: &EntryStat,
    ) -> (MountTable, Option<(i64, u32)>);
    fn file_handle(&self, at: Entry<'_>) -> Result<u64, Errno>;
    /// udev's `ID_FS_TYPE`, when the device has a database record.
    fn device_format(&self, fd: BorrowedFd<'_>) -> Result<Option<String>, Errno>;
    fn rotational(&self, fd: BorrowedFd<'_>) -> Option<bool>;
    fn holders(&self, folder: BorrowedFd<'_>, leftovers: &[OsString]) -> Option<Holder>;
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
        match mount_id(fd) {
            Some(id) => mount_of(id),
            None => MountTable::current()
                .mount_at(&opened_path(fd).ok()?)
                .map(|(mount_type, site)| (mount_type.clone(), site.clone())),
        }
    }

    fn folder_mount(&self, path: &Path, id: Option<u64>) -> Option<(u64, MountType)> {
        assert_off_main_thread();
        let id = match id {
            Some(id) => id,
            // Before Linux 5.8.
            None => {
                let folder =
                    rustix::fs::open(path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty()).ok()?;
                let real = opened_path(folder.as_fd()).ok()?;
                let at = |table: &MountTable| table.mount_for(&real).map(|mount| mount.mount_id);
                at(&mount_watch::latest()).or_else(|| at(&MountTable::current()))?
            }
        };
        mount_of(id).map(|(mount_type, _)| (id, mount_type))
    }

    fn watch(
        &self,
        file: &gio::File,
        watched: Watched,
        flags: gio::FileMonitorFlags,
    ) -> Result<gio::FileMonitor, glib::Error> {
        assert_off_main_thread();
        match watched {
            Watched::Folder => file.monitor_directory(flags, gio::Cancellable::NONE),
            Watched::File => file.monitor_file(flags, gio::Cancellable::NONE),
        }
    }

    fn unwatch(&self, _file: &gio::File, monitor: gio::FileMonitor) {
        assert_off_main_thread();
        monitor.cancel();
    }

    fn current_mount_type(&self, fd: BorrowedFd<'_>) -> Option<MountType> {
        assert_off_main_thread();
        let table = MountTable::current();
        let mount = match mount_id(fd) {
            Some(id) => table.mount_of(id),
            None => table.mount_at(&opened_path(fd).ok()?),
        };
        mount.map(|(mount_type, _)| mount_type.clone())
    }

    fn file_handle_inputs(
        &self,
        _at: Entry<'_>,
        stat: &EntryStat,
    ) -> (MountTable, Option<(i64, u32)>) {
        (mount_watch::latest(), stat.identity.birth)
    }

    fn file_handle(&self, at: Entry<'_>) -> Result<u64, Errno> {
        native_fs::file_handle(at.dir, at.name)
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

    fn holders(&self, folder: BorrowedFd<'_>, leftovers: &[OsString]) -> Option<Holder> {
        assert_off_main_thread();
        holders::scan(folder, leftovers, &holders::processes())
    }
}

fn mount_of(id: u64) -> Option<(MountType, MountSite)> {
    let owned = |(mount_type, site): (&MountType, &MountSite)| (mount_type.clone(), site.clone());
    mount_watch::latest()
        .mount_of(id)
        .map(owned)
        .or_else(|| MountTable::current().mount_of(id).map(owned))
}

fn mount_id(fd: BorrowedFd<'_>) -> Option<u64> {
    rustix::fs::statx(
        fd,
        "",
        AtFlags::EMPTY_PATH | AtFlags::STATX_DONT_SYNC,
        StatxFlags::MNT_ID,
    )
    .ok()
    .filter(|stat| StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID))
    .map(|stat| stat.stx_mnt_id)
}

#[cfg(test)]
mod tests;
