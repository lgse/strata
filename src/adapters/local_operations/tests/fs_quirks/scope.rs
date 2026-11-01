// SPDX-License-Identifier: MIT

use super::*;

pub(crate) struct FsQuirks {
    scope: Arc<Scope>,
}

impl FsQuirks {
    pub(crate) fn install(root: &Path, quirks: Quirks) -> Self {
        let mount_id =
            rustix::fs::statx(rustix::fs::CWD, root, AtFlags::empty(), StatxFlags::MNT_ID)
                .ok()
                .filter(|stat| {
                    StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID)
                })
                .map(|stat| stat.stx_mnt_id);
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let scope = Arc::new(Scope {
            id: NEXT_SCOPE.fetch_add(1, Ordering::Relaxed),
            host: host_mount_type(mount_id, &root),
            root,
            mount_id,
            own_device: quirks.own_device,
            naming: Mutex::new(()),
            state: Mutex::new(State {
                quirks,
                calls: Vec::new(),
                inodes: HashMap::new(),
                births: HashMap::new(),
                reset_births: HashMap::new(),
                link_names: HashMap::new(),
                links_made: 0,
                stale: HashMap::new(),
                written: HashSet::new(),
                written_times: HashMap::new(),
                renumbered: HashSet::new(),
                writebacks_lost: 0,
                reported_losses: HashMap::new(),
                opened_names: Vec::new(),
                read_only_unseen: HashSet::new(),
                seen_change_times: HashMap::new(),
                seen_times: HashMap::new(),
                listings: HashMap::new(),
                own_changes: 0,
                unplugged: false,
                serverino_disabled: false,
                holds: HashMap::new(),
                held_paths: HashMap::new(),
                closed_holds: Vec::new(),
                leftovers: Vec::new(),
                silly_renames: 0,
                cached: HashMap::new(),
                created: HashSet::new(),
                made_up: HashMap::new(),
                recorded_births: HashMap::new(),
                latest_birth: SystemTime::UNIX_EPOCH,
            }),
        });
        scopes().push(scope.clone());
        Self { scope }
    }

    pub(crate) fn key(&self) -> MountKey {
        MountKey::Test(self.scope.id)
    }

    /// Loses a writeback in this scope, from a racer: as errseq does,
    /// `syncfs` reports it once through each folder opened before, and
    /// never through one opened after.
    pub(crate) fn writeback_loser(&self) -> impl Fn() + Send + Sync + 'static {
        let scope = self.scope.clone();
        move || scope.state().writebacks_lost += 1
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.scope.state().calls.clone()
    }

    pub(crate) fn reader(&self) -> impl Fn() -> Vec<Call> + Send + Sync + 'static {
        let scope = self.scope.clone();
        move || scope.state().calls.clone()
    }

    pub(crate) fn count(&self, op: Op) -> usize {
        self.scope
            .state()
            .calls
            .iter()
            .filter(|call| call.op == op)
            .count()
    }

    pub(crate) fn clear_calls(&self) {
        self.scope.state().calls.clear();
    }

    pub(crate) fn replug(&self) {
        self.scope.state().unplugged = false;
    }

    pub(crate) fn used_name(&self, pattern: &str) -> bool {
        self.scope.state().calls.iter().any(|call| {
            let names: Vec<&OsStr> = call.names.iter().map(OsString::as_os_str).collect();
            matches(pattern, &names)
        })
    }
}

impl Drop for FsQuirks {
    fn drop(&mut self) {
        scopes().retain(|scope| !Arc::ptr_eq(scope, &self.scope));
        forget_test_mount_capabilities(self.scope.id);
    }
}

pub(crate) struct HoldGuard {
    pub(super) scope: Arc<Scope>,
    pub(super) key: (u64, u64),
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        let mut state = self.scope.state();
        let Some(count) = state.holds.get_mut(&self.key) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        state.holds.remove(&self.key);
        let now = clock_for_test();
        if let Some(path) = state.held_paths.remove(&self.key) {
            state.closed_holds.push((path, now));
        }
        for leftover in &mut state.leftovers {
            if leftover.key == self.key {
                leftover.unheld_since.get_or_insert(now);
            }
        }
    }
}

pub(super) fn take_racer(
    racers: &mut Vec<(Op, &'static str, usize, Racer)>,
    op: Op,
    names: &[&OsStr],
) -> Option<Racer> {
    let index = racers
        .iter()
        .position(|(racer_op, pattern, _, _)| *racer_op == op && matches(pattern, names))?;
    if racers[index].2 == 0 {
        return Some(racers.remove(index).3);
    }
    racers[index].2 -= 1;
    None
}

pub(super) fn still_open(fd: RawFd, name: &Path, key: (u64, u64)) -> bool {
    use std::os::unix::fs::MetadataExt;
    let link = format!("/proc/self/fd/{fd}");
    std::fs::read_link(&link).is_ok_and(|opened| opened == name)
        && std::fs::metadata(&link).is_ok_and(|file| (file.dev(), file.ino()) == key)
}

/// A bare `*` also matches calls that name nothing.
pub(super) fn matches(pattern: &str, names: &[&OsStr]) -> bool {
    pattern == "*"
        || names.iter().any(|name| match pattern.strip_suffix('*') {
            Some(prefix) => name.as_encoded_bytes().starts_with(prefix.as_bytes()),
            None => name.as_encoded_bytes() == pattern.as_bytes(),
        })
}

impl Scope {
    pub(super) fn record_open(&self, opened: OwnedFd) -> OwnedFd {
        let mut state = self.state();
        let lost = state.writebacks_lost;
        state.reported_losses.insert(opened.as_raw_fd(), lost);
        opened
    }

    pub(super) fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(super) fn naming(&self) -> Option<MutexGuard<'_, ()>> {
        let compares = self.quirk(|quirks| quirks.case_insensitive || quirks.stored_as.is_some());
        compares.then(|| {
            self.naming
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        })
    }

    pub(super) fn run<T>(
        &self,
        op: Op,
        names: &[&OsStr],
        perform: impl FnOnce(&Self) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        self.run_logged(op, names, false, perform)
    }

    pub(super) fn run_logged<T>(
        &self,
        op: Op,
        names: &[&OsStr],
        forced: bool,
        perform: impl FnOnce(&Self) -> Result<T, Errno>,
    ) -> Result<T, Errno> {
        let racer = {
            let mut state = self.state();
            state.calls.push(Call {
                op,
                names: names.iter().map(|name| name.to_os_string()).collect(),
                forced,
            });
            if state.unplugged {
                return Err(Errno::IO);
            }
            take_racer(&mut state.quirks.before, op, names)
        };
        if let Some(mut racer) = racer {
            racer();
        }
        let fault = {
            let mut state = self.state();
            let mut fault = None;
            if let Some(index) = state
                .quirks
                .fail
                .iter()
                .position(|(fail_op, pattern, _, _)| *fail_op == op && matches(pattern, names))
            {
                let (_, _, skip, errno) = &mut state.quirks.fail[index];
                if *skip == 0 {
                    fault = Some(*errno);
                    state.quirks.fail.remove(index);
                } else {
                    *skip -= 1;
                }
            }
            fault
        };
        if let Some(errno) = fault {
            return Err(errno);
        }
        self.reap();
        let result = perform(self);
        {
            let mut state = self.state();
            if result.is_ok() && op.changes() {
                state.own_changes += 1;
            }
            if let Some((unplug_op, pattern, skip)) = state.quirks.unplugged_after.as_mut()
                && *unplug_op == op
                && matches(pattern, names)
            {
                if *skip == 0 {
                    state.quirks.unplugged_after = None;
                    state.unplugged = true;
                } else {
                    *skip -= 1;
                }
            }
        }
        let racer = take_racer(&mut self.state().quirks.after, op, names);
        if let Some(mut racer) = racer {
            racer();
        }
        if result.is_ok() {
            let mut state = self.state();
            if let Some(index) = state
                .quirks
                .lost_reply
                .iter()
                .position(|(lost_op, pattern, _)| *lost_op == op && matches(pattern, names))
            {
                return Err(state.quirks.lost_reply.remove(index).2);
            }
        }
        result
    }

    pub(super) fn quirk<T>(&self, read: impl FnOnce(&Quirks) -> T) -> T {
        read(&self.state().quirks)
    }
}
