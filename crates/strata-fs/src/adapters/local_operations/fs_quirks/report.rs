// SPDX-License-Identifier: MIT

use super::*;

impl Scope {
    pub fn cached(&self, mut stat: EntryStat, forced: bool) -> EntryStat {
        let mut state = self.state();
        let Some(kept_for) = state.quirks.cached_change_times else {
            return stat;
        };
        let now = clock_for_test();
        let seen = state
            .seen_change_times
            .entry((stat.identity.device, stat.identity.inode))
            .or_insert((stat.ctime, now));
        if forced
            || now
                .duration_since(seen.1)
                .map_or(true, |age| age >= kept_for)
        {
            *seen = (stat.ctime, now);
        }
        stat.ctime = seen.0;
        stat
    }

    pub fn cached_stamp(&self, mut stamp: FolderStamp, forced: bool) -> FolderStamp {
        let mut state = self.state();
        let cached_times = state.quirks.cached_times.map(|times| {
            if stamp.kind == FileType::Directory {
                times.folders
            } else {
                times.files
            }
        });
        let (kept_for, ignores_force) = match (state.quirks.daemon_cache, cached_times) {
            (Some(daemon), _) => (daemon, true),
            (None, Some(kept_for)) => (kept_for, false),
            (None, None) => return stamp,
        };
        let now = state.fetch();
        let fetched = Seen {
            modified: stamp.modified,
            changed: stamp.changed,
            fetch: now,
        };
        let seen = state
            .seen_times
            .entry((stamp.device, stamp.inode))
            .or_insert(fetched);
        let forced_through = forced && !ignores_force;
        if forced_through || !seen.fetch.kept(kept_for, now) {
            *seen = fetched;
        }
        (stamp.modified, stamp.changed) = (seen.modified, seen.changed);
        stamp
    }

    pub fn listed(&self, folder: &Path, listing: Listed) -> Listed {
        let mut state = self.state();
        let Some(cached) = state.quirks.cached_listing else {
            return listing;
        };
        let now = state.fetch();
        let entry = state
            .listings
            .entry(folder.to_path_buf())
            .or_insert_with(|| (listing.clone(), now));
        if !entry.1.kept(cached.kept_for, now) {
            *entry = (listing, now);
        } else if cached.renewed_by_use {
            entry.1.at = now.at;
        }
        entry.0.clone()
    }

    pub fn used(&self, folder: &Path) {
        let mut state = self.state();
        if state
            .quirks
            .cached_listing
            .is_some_and(|cached| cached.renewed_by_use)
            && let Some((_, seen)) = state.listings.get_mut(folder)
        {
            seen.at = clock_for_test();
        }
    }

    pub fn reported(&self, path: Option<PathBuf>, mut stat: EntryStat) -> EntryStat {
        let key = (stat.identity.device, stat.identity.inode);
        if let Some(device) = path.as_deref().and_then(own_device) {
            stat.identity.device = device;
        }
        if let Some(birth) = stat.identity.birth.filter(|_| stat.is_directory())
            && self.keeps_folder_births_its_base_restamps()
        {
            stat.identity.birth = Some(*first_folder_births().entry(key).or_insert(birth));
        }
        let mut state = self.state();
        let cached = path
            .as_ref()
            .and_then(|path| state.cached.get(path).copied());
        state.report_identity(path.as_deref(), key, &mut stat);
        state.report_attributes(path.as_deref(), &mut stat);
        state.report_reused_inode(path, &mut stat);
        state.report_times(key, &mut stat);
        if let Some((inode, birth)) = cached {
            stat.identity.inode = inode;
            stat.identity.birth = birth;
        }
        stat
    }

    pub fn reported_stamp(&self, path: &Path, stamp: FolderStamp) -> FolderStamp {
        let real = path.canonicalize().ok();
        let stat = EntryStat {
            identity: LocalFileIdentity {
                device: stamp.device,
                inode: stamp.inode,
                kind: stamp.kind,
                birth: stamp.birth,
                handle: None,
            },
            kind: stamp.kind,
            mode: stamp.mode & 0o7777,
            nlink: 1,
            size: stamp.size,
            blocks: 0,
            atime: stamp.modified,
            mtime: stamp.modified,
            ctime: stamp.changed,
            uid: 0,
            gid: 0,
        };
        if let Some(real) = real.as_deref().filter(|_| stat.is_directory()) {
            self.used(real);
        }
        let stat = self.reported(real, stat);
        FolderStamp {
            modified: stat.mtime,
            changed: stat.ctime,
            device: stat.identity.device,
            inode: stat.identity.inode,
            birth: stat.identity.birth,
            mount_id: (!self.quirk(|quirks| quirks.no_mount_ids)).then_some(u64::MAX - self.id),
            ..stamp
        }
    }

    pub fn mark_written(&self, fd: BorrowedFd<'_>) {
        if self.quirk(|quirks| quirks.times_reset_at_close)
            && let Ok(stat) = System.stat_fd(fd)
        {
            self.state()
                .written
                .insert((stat.identity.device, stat.identity.inode));
        }
    }

    pub fn opened_for_writing(&self, fd: BorrowedFd<'_>) {
        if self.quirk(|quirks| quirks.stale_written_times)
            && let Ok(stat) = System.stat_fd(fd)
        {
            self.state().written_times.insert(
                (stat.identity.device, stat.identity.inode),
                WrittenTime {
                    birth: stat.identity.birth,
                    modified: stat.mtime,
                },
            );
        }
    }

    pub fn with_written_time(&self, mut stat: EntryStat) -> EntryStat {
        let key = (stat.identity.device, stat.identity.inode);
        if let Some(written) = self.state().written_times.get(&key)
            && written.birth == stat.identity.birth
        {
            stat.mtime = written.modified;
        }
        stat
    }

    pub fn revalidate_written_time(&self, stat: &EntryStat) {
        let key = (stat.identity.device, stat.identity.inode);
        if let Some(written) = self.state().written_times.get_mut(&key)
            && written.birth == stat.identity.birth
        {
            written.modified = stat.mtime;
        }
    }

    pub fn closing(&self, fd: BorrowedFd<'_>) {
        let written = System.stat_fd(fd).is_ok_and(|stat| {
            let key = (stat.identity.device, stat.identity.inode);
            let mut state = self.state();
            state.written_times.remove(&key);
            state.written.remove(&key)
        });
        if written {
            let now = rustix::fs::Timespec {
                tv_sec: 0,
                tv_nsec: rustix::fs::UTIME_NOW,
            };
            let _ = rustix::fs::futimens(
                fd,
                &rustix::fs::Timestamps {
                    last_access: now,
                    last_modification: now,
                },
            );
        }
    }
}

impl State {
    /// The birth time and inode number an entry reports, before any name
    /// pattern applies.
    fn report_identity(&mut self, path: Option<&Path>, key: (u64, u64), stat: &mut EntryStat) {
        if self.quirks.births == Births::Recorded
            && stat.identity.birth.is_none_or(|birth| birth == (0, 0))
        {
            stat.identity.birth = self.recorded_birth(key);
        }
        if let Some(&birth) = self.reset_births.get(&key).filter(|_| stat.is_directory()) {
            stat.identity.birth = Some(birth);
        }
        if let Some(&(inode, reported)) = path.and_then(|path| self.link_names.get(path))
            && inode == stat.identity.inode
        {
            stat.identity.inode = reported;
        }
        if path.is_some_and(|path| self.renumbered.contains(path)) {
            stat.identity.inode = !stat.identity.inode;
        }
    }

    /// The link count, mode, group and size an entry reports.
    fn report_attributes(&self, path: Option<&Path>, stat: &mut EntryStat) {
        if self.quirks.single_link_count {
            stat.nlink = 1;
        }
        if let (Some((pattern, mode)), Some(name)) =
            (self.quirks.reported_mode, path.and_then(Path::file_name))
            && matches(pattern, &[name])
        {
            stat.mode = mode;
        }
        if self.quirks.foreign_group {
            stat.gid = stat.gid.wrapping_add(1);
        }
        if self.quirks.virtual_sizes && stat.kind == FileType::RegularFile {
            stat.size = 0;
        }
    }

    /// The first inode number seen at a name matching `reused_inodes`.
    fn report_reused_inode(&mut self, path: Option<PathBuf>, stat: &mut EntryStat) {
        if let (Some(pattern), Some(path)) = (self.quirks.reused_inodes, path)
            && path
                .file_name()
                .is_some_and(|name| matches(pattern, &[name]))
        {
            stat.identity.inode = *self
                .inodes
                .entry(path.clone())
                .or_insert(stat.identity.inode);
            stat.identity.birth = Some((1 + *self.births.entry(path).or_default(), 0));
        }
    }

    /// The times an entry reports, as the clock ticks and the family keeps them.
    fn report_times(&mut self, key: (u64, u64), stat: &mut EntryStat) {
        if let Some(tick) = self.quirks.coarse_clock.and_then(timespec) {
            stat.mtime = stat.mtime.min(tick);
            stat.ctime = stat.ctime.min(tick);
            stat.identity.birth = stat.identity.birth.map(|birth| birth.min(tick));
        }
        if self.quirks.births == Births::MadeUp && !self.created.contains(&key) {
            let earliest = stat.atime.min(stat.mtime).min(stat.ctime);
            stat.identity.birth = Some(*self.made_up.entry(key).or_insert(earliest));
        }
        if self.quirks.no_birth_time {
            stat.identity.birth = None;
        }
        if self.quirks.ctime_is_mtime {
            stat.ctime = stat.mtime;
        }
    }
}
