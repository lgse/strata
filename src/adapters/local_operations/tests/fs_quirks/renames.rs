// SPDX-License-Identifier: MIT

use super::*;

impl Scope {
    pub(super) fn rename(
        &self,
        from: Entry<'_>,
        to: Entry<'_>,
        flags: RenameFlags,
    ) -> Result<(), Errno> {
        if let Some(folder) = path_of(from.dir).map(|dir| dir.join(from.name))
            && self.refuses_folder_rename(&folder)
        {
            return Err(Errno::ACCESS);
        }
        let _naming = self.naming();
        let (rename_flags, exchange, over_empty) = self.quirk(|quirks| {
            (
                quirks.rename_flags,
                quirks.exchange,
                quirks.rename_over_empty_dir,
            )
        });
        if flags.contains(RenameFlags::NOREPLACE) {
            if self.occupied(to) || self.stale(to) {
                return Err(Errno::EXIST);
            }
            if let Support::Rejected(error) = rename_flags {
                return Err(error);
            }
        }
        if let Some(errno) = self.refusal(to.name) {
            return Err(errno);
        }
        let stored = self.stored(to.name);
        let to = Entry {
            dir: to.dir,
            name: &stored,
        };
        if flags.contains(RenameFlags::EXCHANGE) {
            match exchange {
                Exchange::Supported
                    if System
                        .statfs(to.dir)
                        .is_some_and(|stat| stat.kind == FUSE_SUPER_MAGIC) =>
                {
                    return Err(Errno::INVAL);
                }
                Exchange::Supported => {}
                Exchange::Rejected => return Err(Errno::INVAL),
                Exchange::IgnoredOverwrites => {
                    return System.rename(from, to, RenameFlags::empty());
                }
            }
        }
        if flags.is_empty() {
            let source_is_directory = System
                .stat(from, false)
                .is_ok_and(|stat| stat.is_directory());
            let target_is_empty_directory =
                System.stat(to, false).is_ok_and(|stat| stat.is_directory())
                    && System.names(to).is_ok_and(|names| names.is_empty());
            if source_is_directory && target_is_empty_directory {
                match over_empty {
                    OverEmptyDirectory::Supported => {}
                    OverEmptyDirectory::NotEmpty => return Err(Errno::NOTEMPTY),
                    OverEmptyDirectory::Eperm => return Err(Errno::PERM),
                }
            }
            self.cut_rename_over_name(to)?;
            if let Some(alias) = self.alias(to) {
                return System.rename(
                    from,
                    Entry {
                        dir: to.dir,
                        name: &alias,
                    },
                    flags,
                );
            }
        }
        System.rename(from, to, flags)
    }

    pub(super) fn cut_rename_over_name(&self, to: Entry<'_>) -> Result<(), Errno> {
        if !self.quirk(|quirks| quirks.cut_rename_over_names) {
            return Ok(());
        }
        let Ok(existing) = System.stat(to, false) else {
            return Ok(());
        };
        let temporary = format!("{}.ntfs-3g-0000000001", to.name.to_string_lossy())
            .encode_utf16()
            .collect::<Vec<_>>();
        if temporary.len() <= 255 {
            return Ok(());
        }
        let stored = match temporary.len() % 256 {
            0 => return Err(Errno::INVAL),
            kept => OsString::from(String::from_utf16_lossy(&temporary[..kept])),
        };
        let stray = Entry {
            dir: to.dir,
            name: &stored,
        };
        let _ = if existing.is_directory() {
            System.mkdir(stray, Mode::from_raw_mode(0o755))
        } else {
            System.link(to, stray)
        };
        Ok(())
    }

    pub(super) fn link(&self, from: Entry<'_>, to: Entry<'_>) -> Result<(), Errno> {
        let _naming = self.naming();
        if self.invalid(to.name).is_some() {
            return Err(Errno::PERM);
        }
        if let Some(errno) = self.too_long(to.name, |quirks| quirks.name_limit) {
            return Err(errno);
        }
        let stored = self.stored(to.name);
        let to = Entry {
            dir: to.dir,
            name: &stored,
        };
        if self.occupied(to) {
            return Err(self.collision());
        }
        if let Support::Rejected(error) = self.quirk(|quirks| quirks.links) {
            return Err(error);
        }
        System.link(from, to).map_err(|error| {
            if error == Errno::EXIST {
                self.collision()
            } else {
                error
            }
        })?;
        if self.quirk(|quirks| quirks.link_inodes)
            && let (Some(path), Ok(stat)) = (path_of(to.dir), System.stat(to, false))
        {
            let mut state = self.state();
            state.links_made += 1;
            let reported = u64::MAX - state.links_made;
            state
                .link_names
                .insert(path.join(to.name), (stat.identity.inode, reported));
        }
        Ok(())
    }

    pub(super) fn renamed(&self, from: Entry<'_>, to: Entry<'_>, flags: RenameFlags) {
        if self.quirk(|quirks| quirks.renames_reset_folder_births)
            && let (Ok(folder), Some(now)) = (System.stat_fd(to.dir), timespec(SystemTime::now()))
        {
            self.state()
                .reset_births
                .insert((folder.identity.device, folder.identity.inode), now);
        }
        let (Some(from), Some(to)) = (
            path_of(from.dir).map(|dir| dir.join(from.name)),
            path_of(to.dir).map(|dir| dir.join(to.name)),
        ) else {
            return;
        };
        let mut state = self.state();
        let cached = state.cached.remove(&from);
        state.cached.remove(&to);
        if let Some(cached) = cached {
            state.cached.insert(to.clone(), cached);
        }
        let renumbered = state.renumbered.remove(&from);
        let displaced_renumbered = state.renumbered.remove(&to);
        if renumbered {
            state.renumbered.insert(to.clone());
        }
        if displaced_renumbered && flags.contains(RenameFlags::EXCHANGE) {
            state.renumbered.insert(from.clone());
        }
        let moved = state.link_names.remove(&from);
        let displaced = state.link_names.remove(&to);
        if let Some(moved) = moved {
            state.link_names.insert(to, moved);
        }
        if let Some(displaced) = displaced.filter(|_| flags.contains(RenameFlags::EXCHANGE)) {
            state.link_names.insert(from, displaced);
        }
    }

    pub(super) fn inode_kept_on_lost_reply(&self, to: Entry<'_>) -> Option<u64> {
        if !self.quirk(|quirks| quirks.lost_renames_keep_inodes) {
            return None;
        }
        System.stat(to, false).ok().map(|stat| stat.identity.inode)
    }

    pub(super) fn keep_inode(&self, to: Entry<'_>, kept: u64) {
        if let (Some(dir), Ok(stat)) = (path_of(to.dir), System.stat(to, false))
            && stat.identity.inode != kept
        {
            self.state()
                .link_names
                .insert(dir.join(to.name), (stat.identity.inode, kept));
        }
    }

    pub(super) fn unlinked(&self, at: Entry<'_>) {
        if let Some(path) = path_of(at.dir) {
            let path = path.join(at.name);
            let mut state = self.state();
            state.link_names.remove(&path);
            state.renumbered.remove(&path);
            state.cached.remove(&path);
        }
    }
}
