// SPDX-License-Identifier: MIT

use super::*;

pub fn held_by_this_process(device: u64, inode: u64) -> bool {
    refers_to(device, inode, true)
}

pub fn refers_to(device: u64, inode: u64, o_path: bool) -> bool {
    let path_only = |fd: &std::fs::DirEntry| {
        let info = format!("/proc/self/fdinfo/{}", fd.file_name().to_string_lossy());
        std::fs::read_to_string(info)
            .ok()
            .and_then(|info| {
                let flags = info.lines().find_map(|line| line.strip_prefix("flags:"))?;
                u32::from_str_radix(flags.trim(), 8).ok()
            })
            .is_some_and(|flags| flags & OFlags::PATH.bits() != 0)
    };
    std::fs::read_dir("/proc/self/fd").is_ok_and(|fds| {
        fds.filter_map(Result::ok).any(|fd| {
            (o_path || !path_only(&fd))
                && rustix::fs::stat(fd.path())
                    .is_ok_and(|open| (open.st_dev, open.st_ino) == (device, inode))
        })
    })
}

impl Aside {
    pub fn held_here(self, (device, inode): (u64, u64)) -> bool {
        refers_to(device, inode, matches!(self, Self::Nfs))
    }

    pub fn name(self, inode: u64, count: u64) -> Option<OsString> {
        let name = match self {
            Self::Nfs => format!(".nfs{inode:016x}{count:08x}"),
            Self::Smb => format!(".__smbfile_silly{count:x}"),
            Self::Fuse => format!(".fuse_hidden{:08x}{:08x}", inode as u32, count as u32),
            Self::Unlisted(_) => return None,
        };
        Some(OsString::from(name))
    }
}

impl Scope {
    pub fn refuses_folder_rename(&self, folder: &Path) -> bool {
        let Some(lingers) = self.quirk(|quirks| quirks.folder_renames_refused_below) else {
            return false;
        };
        if !std::fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.is_dir()) {
            return false;
        }
        let now = clock_for_test();
        let state = self.state();
        let below = |path: &Path| path != folder && path.starts_with(folder);
        state
            .holds
            .keys()
            .filter_map(|key| state.held_paths.get(key))
            .any(|path| below(path))
            || state.closed_holds.iter().any(|(path, closed)| {
                below(path) && now.duration_since(*closed).unwrap_or_default() < lingers
            })
    }

    pub fn reap(&self) {
        let mut state = self.state();
        let Some(silly) = state.quirks.silly_rename else {
            return;
        };
        let now = clock_for_test();
        let State {
            holds, leftovers, ..
        } = &mut *state;
        leftovers.retain_mut(|leftover| {
            if holds.contains_key(&leftover.key) || silly.aside.held_here(leftover.key) {
                leftover.unheld_since = None;
                return true;
            }
            let since = *leftover.unheld_since.get_or_insert(now);
            if now.duration_since(since).unwrap_or_default() < silly.lingers {
                return true;
            }
            if let Some(name) = &leftover.name {
                let _ = std::fs::remove_file(leftover.folder.join(name));
            }
            false
        });
    }

    pub fn unlink_held(&self, at: Entry<'_>, flags: AtFlags) -> Option<Result<(), Errno>> {
        let silly = self.quirk(|quirks| quirks.silly_rename)?;
        let folder = path_of(at.dir)?;
        if flags.contains(AtFlags::REMOVEDIR) {
            return self.unlink_held_folder(at, silly.aside);
        }
        let stat = System
            .stat(at, false)
            .ok()
            .filter(|stat| stat.kind == FileType::RegularFile)?;
        let key = (stat.identity.device, stat.identity.inode);
        let folder_key = System.stat_fd(at.dir).ok()?.identity;
        let mut state = self.state();
        let leftover = state.leftovers.iter().position(|leftover| {
            leftover.folder == folder && leftover.name.as_deref() == Some(at.name)
        });
        let held = || state.holds.contains_key(&key) || silly.aside.held_here(key);
        match (leftover, silly.aside) {
            (Some(_), Aside::Nfs) => return Some(Err(Errno::BUSY)),
            (Some(_), Aside::Smb) => return Some(Err(Errno::NOENT)),
            (Some(_), _) => {}
            (None, _) if !held() => return None,
            (None, _) => {}
        }
        let name = match state.free_silly_name(at, silly.aside, key.1) {
            Ok(name) => name,
            Err(error) => return Some(Err(error)),
        };
        let unlinked = match &name {
            Some(name) => System.rename(at, Entry { dir: at.dir, name }, RenameFlags::empty()),
            None => System.unlink(at, AtFlags::empty()),
        };
        if unlinked.is_ok() {
            state.record_leftover(
                leftover,
                SillyLeftover {
                    folder,
                    folder_key: (folder_key.device, folder_key.inode),
                    name,
                    key,
                    unheld_since: None,
                },
            );
        }
        Some(unlinked)
    }

    /// A folder whose removal an unlisted leftover below it refuses.
    fn unlink_held_folder(&self, at: Entry<'_>, aside: Aside) -> Option<Result<(), Errno>> {
        let Aside::Unlisted(errno) = aside else {
            return None;
        };
        let target = System.stat(at, false).ok()?.identity;
        let held_below = self
            .state()
            .leftovers
            .iter()
            .any(|leftover| leftover.folder_key == (target.device, target.inode));
        held_below.then_some(Err(errno))
    }
}

impl State {
    /// The next name `aside` sets a file aside under that nothing in `at`'s
    /// folder holds, or none where it is set aside unlisted.
    fn free_silly_name(
        &mut self,
        at: Entry<'_>,
        aside: Aside,
        inode: u64,
    ) -> Result<Option<OsString>, Errno> {
        loop {
            self.silly_renames += 1;
            let name = aside.name(inode, self.silly_renames);
            let Some(silly_name) = &name else {
                return Ok(name);
            };
            match rustix::fs::statat(at.dir, silly_name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => {}
                Err(Errno::NOENT) => return Ok(name),
                Err(error) => return Err(error),
            }
        }
    }

    /// Names the leftover at `index` after it again, or records a new one.
    fn record_leftover(&mut self, index: Option<usize>, set_aside: SillyLeftover) {
        match index {
            Some(index) => self.leftovers[index].name = set_aside.name,
            None => self.leftovers.push(set_aside),
        }
    }
}
