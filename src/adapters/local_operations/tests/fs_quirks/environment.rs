// SPDX-License-Identifier: MIT

use super::*;

pub(crate) struct Scoped;

pub(crate) static SCOPED: Scoped = Scoped;

impl FsEnvironment for Scoped {
    fn can_trash(&self, path: &Path) -> bool {
        assert_off_main_thread();
        match scope_for_path(path) {
            Some(scope) => scope
                .run(Op::CanTrash, &[], |scope| {
                    Ok(match scope.quirk(|quirks| quirks.trash) {
                        Trash::Supported => System.can_trash(path),
                        Trash::Unsupported => false,
                        Trash::FalseWithoutError
                        | Trash::SucceedsWithoutMoving
                        | Trash::Refused
                        | Trash::ProtocolRefused => true,
                    })
                })
                .unwrap_or(false),
            None => System.can_trash(path),
        }
    }

    fn trash(&self, path: &Path, cancellable: &gio::Cancellable) -> Result<bool, glib::Error> {
        assert_off_main_thread();
        let Some(scope) = scope_for_path(path) else {
            return System.trash(path, cancellable);
        };
        let name = path.file_name().unwrap_or_default();
        let mut outcome = Ok(true);
        let ran = scope.run(Op::Trash, &[name], |scope| {
            let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if scope.refuses_folder_rename(&real) {
                return Err(Errno::ACCESS);
            }
            outcome = match scope.quirk(|quirks| quirks.trash) {
                Trash::Supported => System.trash(path, cancellable),
                Trash::Unsupported | Trash::Refused => Err(glib::Error::new(
                    gio::IOErrorEnum::NotSupported,
                    "Trash is not supported here",
                )),
                Trash::ProtocolRefused => Err(glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "The backend refused the Trash protocol",
                )),
                Trash::FalseWithoutError => Ok(false),
                Trash::SucceedsWithoutMoving => Ok(true),
            };
            Ok(())
        });
        if let Err(errno) = ran {
            let code = gio::io_error_from_errno(errno.raw_os_error());
            return Err(glib::Error::new(code, &errno.to_string()));
        }
        outcome
    }

    fn mount_type(&self, fd: BorrowedFd<'_>) -> Option<(MountType, MountSite)> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope.mount(),
            None => System.mount_type(fd),
        }
    }

    fn folder_mount(&self, path: &Path, id: Option<u64>) -> Option<(u64, MountType)> {
        assert_off_main_thread();
        let scope = match id {
            Some(id) => scopes()
                .iter()
                .find(|scope| u64::MAX - scope.id == id)
                .cloned(),
            None => scope_containing(path),
        };
        match scope {
            Some(scope) => scope
                .mount()
                .map(|(mount_type, _)| (u64::MAX - scope.id, mount_type)),
            None => System.folder_mount(path, id),
        }
    }

    fn watch(
        &self,
        file: &gio::File,
        watched: Watched,
        flags: gio::FileMonitorFlags,
    ) -> Result<gio::FileMonitor, glib::Error> {
        assert_off_main_thread();
        let path = file.path();
        let Some((scope, path)) = path
            .as_deref()
            .and_then(|path| Some((scope_containing(path)?, path)))
        else {
            return System.watch(file, watched, flags);
        };
        let mut monitor = None;
        let ran = scope.run(Op::Watch, &[path.file_name().unwrap_or_default()], |_| {
            monitor = Some(System.watch(file, watched, flags));
            Ok(())
        });
        match (ran, monitor) {
            (Ok(()), Some(monitor)) => monitor,
            (Err(errno), _) => Err(glib::Error::new(
                gio::io_error_from_errno(errno.raw_os_error()),
                &errno.to_string(),
            )),
            (Ok(()), None) => unreachable!("a call that ran watched the file"),
        }
    }

    fn unwatch(&self, file: &gio::File, monitor: gio::FileMonitor) {
        assert_off_main_thread();
        let path = file.path();
        if let Some((scope, path)) = path
            .as_deref()
            .and_then(|path| Some((scope_containing(path)?, path)))
        {
            let _ = scope.run(Op::Unwatch, &[path.file_name().unwrap_or_default()], |_| {
                Ok(())
            });
        }
        System.unwatch(file, monitor);
    }

    fn current_mount_type(&self, fd: BorrowedFd<'_>) -> Option<MountType> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope
                .run(Op::CurrentMountType, &[], |scope| {
                    Ok(scope.current_mount_type())
                })
                .ok()
                .flatten(),
            None => System.current_mount_type(fd),
        }
    }

    fn file_handle_inputs(
        &self,
        at: Entry<'_>,
        stat: &EntryStat,
    ) -> (MountTable, Option<(i64, u32)>) {
        let Some(scope) = scope_for(at.dir) else {
            return System.file_handle_inputs(at, stat);
        };
        let path = path_of(at.dir).map(|dir| dir.join(at.name));
        let reported = scope.reported(path, scope.cached(*stat, false));
        (scope.mounts(), reported.identity.birth)
    }

    fn file_handle(&self, at: Entry<'_>) -> Result<u64, Errno> {
        let made_up = scope_for(at.dir)
            .and_then(|_| System.current_mount_type(at.dir))
            .filter(|base| gives_no_handles(base) || family(&base.fs_type) == Family::Fuse);
        match made_up {
            Some(base) => made_up_handle(at, &base),
            None => System.file_handle(at),
        }
    }

    fn device_format(&self, fd: BorrowedFd<'_>) -> Result<Option<String>, Errno> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope.run(Op::DeviceFormat, &[], |scope| {
                match scope.quirk(|quirks| (quirks.mount_type, quirks.device_format)) {
                    (None, None) => System.device_format(fd),
                    (_, format) => Ok(format.map(str::to_owned)),
                }
            }),
            None => System.device_format(fd),
        }
    }

    fn rotational(&self, fd: BorrowedFd<'_>) -> Option<bool> {
        assert_off_main_thread();
        match scope_for(fd) {
            Some(scope) => scope.quirk(|quirks| quirks.rotational),
            None => System.rotational(fd),
        }
    }

    fn holders(&self, folder: BorrowedFd<'_>, leftovers: &[OsString]) -> Option<Holder> {
        assert_off_main_thread();
        let this_process = || holders::scan(folder, leftovers, &[std::process::id()]);
        match scope_for(folder) {
            Some(scope) => scope
                .run(Op::Holders, &[], |scope| {
                    Ok(scope
                        .quirk(|quirks| quirks.holder.clone())
                        .or_else(this_process))
                })
                .ok()
                .flatten(),
            None => this_process(),
        }
    }
}

pub(crate) fn smb_base(fd: BorrowedFd<'_>) -> bool {
    System
        .current_mount_type(fd)
        .is_some_and(|base| family(&base.fs_type) == Family::Smb)
}

pub(super) fn gives_no_handles(base: &MountType) -> bool {
    base.fs_type == "exfat" || family(&base.fs_type) == Family::Smb
}

/// The base's birth time stands for the generation a reused number changes.
pub(super) fn made_up_handle(at: Entry<'_>, base: &MountType) -> Result<u64, Errno> {
    use std::hash::{Hash, Hasher};
    let flags = if at.name.is_empty() {
        AtFlags::EMPTY_PATH
    } else {
        AtFlags::empty()
    };
    let stat = rustix::fs::statx(
        at.dir,
        at.name,
        flags | AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::TYPE | StatxFlags::INO | StatxFlags::BTIME,
    )?;
    let folder = FileType::from_raw_mode(u32::from(stat.stx_mode)) == FileType::Directory;
    let birth = (StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::BTIME)
        && (!folder || base.identity().folder_births))
        .then_some((stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec));
    let mut digest = std::hash::DefaultHasher::new();
    (stat.stx_dev_major, stat.stx_dev_minor, stat.stx_ino, birth).hash(&mut digest);
    Ok(digest.finish())
}

/// Links as on Linux 6.10 and later, where `AT_EMPTY_PATH` needs no privilege, whatever the
/// host kernel runs: `ProcOnly` and `Unavailable` quirks stand for the older kernels. Tests
/// otherwise count a failed plain link and a `/proc` retry on some hosts and one call on others.
pub(super) fn link_fd_as_since_linux_6_10(
    fd: BorrowedFd<'_>,
    to: Entry<'_>,
    through_proc: bool,
) -> Result<(), Errno> {
    match System.link_fd(fd, to, through_proc) {
        Err(Errno::NOENT) if !through_proc => System.link_fd(fd, to, true),
        result => result,
    }
}
