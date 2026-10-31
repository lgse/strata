// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    os::{
        fd::{AsFd, BorrowedFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::gio::prelude::AppInfoExt;
use rustix::{
    fs::{AtFlags, Mode, OFlags, StatxFlags},
    io::Errno,
};

use super::{
    fs_ops::{Entry, FsOps},
    mount_profile::known_mount,
    native_fs::opened_path,
};
use crate::{
    adapters::volume::SillyNames,
    model::hidden_names::ForeignLeftover,
    services::{Holder, file_activity},
};

const SCAN_BUDGET: Duration = Duration::from_millis(200);

const ANCESTORS: usize = 64;

pub(super) fn find(fs: &dyn FsOps, at: Entry<'_>, leftovers: &[OsString]) -> Option<Holder> {
    let folder = fs
        .open_at(
            at,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            None,
        )
        .ok()?;
    if opened_path(folder.as_fd()).is_ok_and(|path| file_activity::in_use_within(&path)) {
        return Some(Holder::Strata);
    }
    fs.holders(folder.as_fd(), leftovers)
}

pub(super) fn held_inside(holder: Option<&Holder>) -> Option<String> {
    Some(match holder? {
        Holder::Program(program) => rust_i18n::t!(
            "A file in it is still open in %{program}. Close it there and try again.",
            program = program
        )
        .into_owned(),
        Holder::Strata => crate::i18n::tr(
            "A file in it is still in use by Strata. Try again when its other operation or search ends.",
        ),
    })
}

pub(super) fn refused_rename(fs: &dyn FsOps, from: Entry<'_>, errno: Errno) -> Option<String> {
    let (_, capabilities) = known_mount(fs, from.dir);
    capabilities.deferred_close?;
    match errno {
        Errno::BUSY => Some(crate::i18n::tr(
            "It is still open in a program on this or another computer.",
        )),
        Errno::ACCESS
            if capabilities.silly_names != Some(SillyNames::Unlisted)
                && fs.stat(from, false).is_ok_and(|stat| stat.is_directory()) =>
        {
            held_inside(find(fs, from, &[]).as_ref())
        }
        _ => None,
    }
}

pub(super) fn held_elsewhere(silly_names: Option<SillyNames>) -> Option<String> {
    match silly_names? {
        SillyNames::Renamed(ForeignLeftover::Smb) => Some(crate::i18n::tr(
            "A file in it is still open in a program on this or another computer.",
        )),
        SillyNames::Renamed(_) => Some(crate::i18n::tr(
            "A file in it is still open in another program. Close it and try again.",
        )),
        SillyNames::Unlisted => None,
    }
}

pub(super) fn held(holder: Option<&Holder>) -> String {
    held_inside(holder).unwrap_or_else(|| {
        crate::i18n::tr("A file in it is still open in another program. Close it and try again.")
    })
}

pub(super) fn processes() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

struct Target {
    folder: PathBuf,
    mount_id: u64,
    device: (u32, u32),
    leftovers: HashSet<u64>,
}

impl Target {
    fn read(folder: BorrowedFd<'_>, leftovers: &[OsString]) -> Option<Self> {
        let fields = StatxFlags::INO | StatxFlags::MNT_ID;
        let stat = |name: &OsStr, flags| {
            rustix::fs::statx(folder, name, flags | AtFlags::STATX_DONT_SYNC, fields).ok()
        };
        let own = stat(OsStr::new(""), AtFlags::EMPTY_PATH)?;
        Some(Self {
            folder: opened_path(folder).ok()?,
            mount_id: own.stx_mnt_id,
            device: (own.stx_dev_major, own.stx_dev_minor),
            leftovers: leftovers
                .iter()
                .filter_map(|name| stat(name, AtFlags::SYMLINK_NOFOLLOW))
                .filter(|leftover| leftover.stx_mnt_id == own.stx_mnt_id)
                .map(|leftover| leftover.stx_ino)
                .collect(),
        })
    }

    fn below(&self, path: &Path) -> bool {
        path != self.folder && path.starts_with(&self.folder)
    }

    fn holds(&self, inode: u64) -> bool {
        self.leftovers.is_empty() || self.leftovers.contains(&inode)
    }

    fn opened_by(&self, pid: u32) -> bool {
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
            return false;
        };
        fds.filter_map(Result::ok).any(|fd| {
            std::fs::read_link(fd.path()).is_ok_and(|link| self.below(&link))
                && self.confirms(pid, &fd.file_name())
        })
    }

    fn confirms(&self, pid: u32, fd: &OsStr) -> bool {
        let Ok(info) =
            std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{}", fd.to_string_lossy()))
        else {
            return false;
        };
        let field = |name: &str| {
            info.lines()
                .find_map(|line| line.strip_prefix(name)?.trim().parse::<u64>().ok())
        };
        field("mnt_id:").is_none_or(|mount_id| mount_id == self.mount_id)
            && field("ino:").is_none_or(|inode| self.holds(inode))
    }

    fn mapped_by(&self, pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/maps")).is_ok_and(|maps| self.maps(&maps))
    }

    fn maps(&self, maps: &str) -> bool {
        maps.lines().any(|line| {
            let mut fields = line.splitn(6, ' ').skip(3);
            let (Some(device), Some(inode), Some(path)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return false;
            };
            let device = device.split_once(':').and_then(|(major, minor)| {
                Some((
                    u32::from_str_radix(major, 16).ok()?,
                    u32::from_str_radix(minor, 16).ok()?,
                ))
            });
            self.below(Path::new(path.trim_start()))
                && device == Some(self.device)
                && inode.parse().is_ok_and(|inode| self.holds(inode))
        })
    }
}

pub(super) fn scan(
    folder: BorrowedFd<'_>,
    leftovers: &[OsString],
    processes: &[u32],
) -> Option<Holder> {
    let target = Target::read(folder, leftovers)?;
    if !leftovers.is_empty() && target.leftovers.is_empty() {
        return None;
    }
    let deadline = Instant::now() + SCAN_BUDGET;
    let passes: [fn(&Target, u32) -> bool; 2] = [Target::opened_by, Target::mapped_by];
    for pass in passes {
        for &pid in processes {
            if Instant::now() > deadline {
                return None;
            }
            if pass(&target, pid) {
                return holder(pid);
            }
        }
    }
    None
}

fn holder(pid: u32) -> Option<Holder> {
    if is_strata(pid) {
        return Some(Holder::Strata);
    }
    app_name(pid)
        .or_else(|| {
            let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
            Some(name.trim_end().to_owned()).filter(|name| !name.is_empty())
        })
        .map(Holder::Program)
}

fn is_strata(pid: u32) -> bool {
    let own = std::process::id();
    let mut process = pid;
    for _ in 0..ANCESTORS {
        if process == own {
            return true;
        }
        let Some((parent, group, session)) = parent_group_session(process) else {
            break;
        };
        if parent == own {
            if group == process
                && parent_group_session(own)
                    .is_some_and(|(_, _, own_session)| own_session == session)
            {
                return true;
            }
            break;
        }
        if parent <= 1 {
            break;
        }
        process = parent;
    }
    let executable = |pid: &str| std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    executable(&pid.to_string()).is_some_and(|exe| executable("self") == Some(exe))
}

fn parent_group_session(pid: u32) -> Option<(u32, u32, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, after_name) = stat.rsplit_once(')')?;
    let mut fields = after_name
        .split_whitespace()
        .skip(1)
        .map(|field| field.parse::<u32>().ok());
    Some((fields.next()??, fields.next()??, fields.next()??))
}

fn app_name(pid: u32) -> Option<String> {
    let named = |info: Option<gio_unix::DesktopAppInfo>| info.map(|info| info.name().to_string());
    let cgroup =
        |pid: &str| std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
    app_ids(&cgroup(&pid.to_string()), &cgroup("self"))
        .into_iter()
        .find_map(|id| named(gio_unix::DesktopAppInfo::new(&format!("{id}.desktop"))))
        .or_else(|| {
            let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
            named(gio_unix::DesktopAppInfo::from_filename(
                launched_desktop_file(&environ, pid)?,
            ))
        })
}

pub(super) fn app_ids(cgroup: &str, own: &str) -> Vec<String> {
    let strata: Vec<&str> = units(own).collect();
    units(cgroup)
        .filter(|unit| !strata.contains(unit))
        .filter_map(|unit| {
            let unit = unit.strip_prefix("app-")?;
            let name = if let Some(scope) = unit.strip_suffix(".scope") {
                scope.rsplit_once('-')?.0
            } else if let Some(service) = unit.strip_suffix(".service") {
                service.split_once('@').map_or(service, |(name, _)| name)
            } else {
                unit.strip_suffix(".slice")?
            };
            Some(match name.split_once('-') {
                Some((_, id)) => vec![unescape(id), unescape(name)],
                None => vec![unescape(name)],
            })
        })
        .flatten()
        .collect()
}

fn units(cgroup: &str) -> impl Iterator<Item = &str> {
    cgroup
        .lines()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .flat_map(|path| path.rsplit('/'))
}

fn unescape(escaped: &str) -> String {
    let mut text = Vec::with_capacity(escaped.len());
    let mut bytes = escaped.as_bytes();
    while let Some((&byte, rest)) = bytes.split_first() {
        let code = rest
            .strip_prefix(b"x")
            .and_then(|rest| rest.get(..2))
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match code {
            Some(code) if byte == b'\\' => {
                text.push(code);
                bytes = &rest[3..];
            }
            _ => {
                text.push(byte);
                bytes = rest;
            }
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

pub(super) fn launched_desktop_file(environ: &[u8], pid: u32) -> Option<PathBuf> {
    let mut file = None;
    let mut launched = None;
    for variable in environ.split(|&byte| byte == 0) {
        if let Some(path) = variable.strip_prefix(b"GIO_LAUNCHED_DESKTOP_FILE=") {
            file = Some(PathBuf::from(OsStr::from_bytes(path)));
        } else if let Some(value) = variable.strip_prefix(b"GIO_LAUNCHED_DESKTOP_FILE_PID=") {
            launched = std::str::from_utf8(value).ok()?.parse::<u32>().ok();
        }
    }
    file.filter(|_| launched == Some(pid))
}

#[cfg(test)]
mod tests;
