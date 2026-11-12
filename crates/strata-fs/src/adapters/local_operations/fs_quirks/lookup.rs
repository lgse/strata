// SPDX-License-Identifier: MIT

use super::*;

pub fn path_of(dir: BorrowedFd<'_>) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd())).ok()
}

/// The basename an fd-only call logs; `#<inode> (deleted)` for an unnamed file.
pub fn fd_name(fd: BorrowedFd<'_>) -> OsString {
    path_of(fd)
        .and_then(|path| path.file_name().map(OsStr::to_os_string))
        .unwrap_or_default()
}

pub fn on_fd<T>(
    fd: BorrowedFd<'_>,
    op: Op,
    extra: Option<&OsStr>,
    perform: impl FnOnce(Option<&Scope>) -> Result<T, Errno>,
) -> Result<T, Errno> {
    match scope_for(fd) {
        Some(scope) => {
            let name = fd_name(fd);
            let mut names = vec![name.as_os_str()];
            names.extend(extra);
            scope.run(op, &names, |scope| perform(Some(scope)))
        }
        None => perform(None),
    }
}

/// A data call that moves bytes from `source` into `target`, which fails
/// with `EXDEV` across scopes as it does across mounts.
pub fn between(
    op: Op,
    source: BorrowedFd<'_>,
    target: BorrowedFd<'_>,
    perform: impl FnOnce() -> Result<usize, Errno>,
) -> Result<usize, Errno> {
    let same = same_scope(&scope_for(source), &scope_for(target));
    on_fd(target, op, None, |scope| {
        if !same {
            return Err(Errno::XDEV);
        }
        if let Some(scope) = scope {
            scope.mark_written(target);
        }
        perform()
    })
}

pub fn scope_for(dir: BorrowedFd<'_>) -> Option<Arc<Scope>> {
    let path = path_of(dir)?;
    scopes()
        .iter()
        .filter(|scope| path.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .cloned()
}

pub fn timespec(time: SystemTime) -> Option<(i64, u32)> {
    let since = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some((i64::try_from(since.as_secs()).ok()?, since.subsec_nanos()))
}

pub fn own_device(path: &Path) -> Option<u64> {
    scopes()
        .iter()
        .filter(|scope| scope.own_device && path.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .map(|scope| u64::from(u32::MAX) - scope.id)
}

pub fn gio_device(file: &gio::File, device: Option<u64>) -> Option<u64> {
    file.path()
        .and_then(|path| own_device(&path.canonicalize().unwrap_or(path)))
        .or(device)
}

pub fn scope_for_folder(path: &Path) -> Option<Arc<Scope>> {
    let folder = path.canonicalize().ok()?;
    scopes()
        .iter()
        .filter(|scope| folder.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .cloned()
}

pub fn scope_for_path(path: &Path) -> Option<Arc<Scope>> {
    scope_containing(path.parent()?)
}

pub fn scope_containing(path: &Path) -> Option<Arc<Scope>> {
    let path = path.canonicalize().ok()?;
    scopes()
        .iter()
        .filter(|scope| path.starts_with(&scope.root))
        .max_by_key(|scope| scope.root.as_os_str().len())
        .cloned()
}

pub fn recreated(path: &Path) {
    let (Some(scope), Some(name)) = (scope_for_path(path), path.file_name()) else {
        return;
    };
    if let Some(parent) = path.parent().and_then(|parent| parent.canonicalize().ok()) {
        *scope.state().births.entry(parent.join(name)).or_default() += 1;
    }
    if let Ok(stat) = rustix::fs::lstat(path) {
        let key = (stat.st_dev, stat.st_ino);
        scope.state().recorded_births.remove(&key);
        first_folder_births().remove(&key);
    }
}

/// Whether a row that sends the malformed `name` as it is to the filesystem holding `root` is
/// skipped there: a FUSE daemon gets what a preset over it passes on, and exfat-fuse writes an
/// overlong `/` into its volume as a real one.
pub fn skips_malformed_name_on_fuse(root: &Path, name: &OsStr) -> bool {
    let skipped = name.to_str().is_none()
        && rustix::fs::statfs(root).is_ok_and(|stat| stat.f_type == FUSE_SUPER_MAGIC);
    if skipped {
        eprintln!("skipped on a FUSE base, which a malformed name can corrupt: {name:?}");
    }
    skipped
}

/// Makes whatever is at `path` report another inode number until it is
/// unlinked, as libfuse without `use_ino` and cifs without server inode
/// numbers do once the kernel evicts an entry.
pub fn renumbered(path: &Path) {
    renumber(path);
}

pub fn renumber(path: &Path) -> bool {
    let (Some(scope), Some(name)) = (scope_for_path(path), path.file_name()) else {
        return false;
    };
    let parent = path.parent().and_then(|parent| parent.canonicalize().ok());
    parent.is_some_and(|parent| {
        scope.state().renumbered.insert(parent.join(name));
        true
    })
}

pub fn evict(path: &Path) -> bool {
    let Ok(stat) = rustix::fs::lstat(path) else {
        return false;
    };
    let held = held_by_this_process(stat.st_dev, stat.st_ino);
    let Some(scope) = scope_for_path(path).filter(|_| !held) else {
        return false;
    };
    {
        let mut state = scope.state();
        state.made_up.remove(&(stat.st_dev, stat.st_ino));
        if let (Some(parent), Some(name)) = (
            path.parent().and_then(|parent| parent.canonicalize().ok()),
            path.file_name(),
        ) {
            state.cached.remove(&parent.join(name));
        }
    }
    !scope.server_numbers() && renumber(path)
}

pub fn hold(path: &Path) -> HoldGuard {
    let scope = scope_for_path(path).expect("a scope holding the file");
    let stat = rustix::fs::lstat(path).expect("the held file");
    let key = (stat.st_dev, stat.st_ino);
    let mut state = scope.state();
    *state.holds.entry(key).or_default() += 1;
    if let Ok(path) = path.canonicalize() {
        state.held_paths.insert(key, path);
    }
    drop(state);
    HoldGuard { scope, key }
}

pub fn autodisable_serverino(path: &Path) -> bool {
    let Some(scope) = scope_containing(path) else {
        return false;
    };
    scope.state().serverino_disabled = true;
    if scope.server_numbers() {
        return true;
    }
    let mut folders = vec![scope.root.clone()];
    while let Some(folder) = folders.pop() {
        let Ok(dir) = std::fs::File::open(&folder) else {
            continue;
        };
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(entry.path());
            }
            let name = entry.file_name();
            if let Ok(stat) = System.stat(Entry::new(&dir, &name), false) {
                let stat = scope.reported(Some(entry.path()), stat);
                scope
                    .state()
                    .cached
                    .insert(entry.path(), (stat.identity.inode, stat.identity.birth));
            }
        }
    }
    true
}

pub fn same_scope(left: &Option<Arc<Scope>>, right: &Option<Arc<Scope>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

/// Runs `scoped` as `op` naming `at` in the scope of its folder, or
/// `system` where no scope holds it.
pub fn at_entry<T>(
    at: Entry<'_>,
    op: Op,
    scoped: impl FnOnce(&Scope) -> Result<T, Errno>,
    system: impl FnOnce() -> Result<T, Errno>,
) -> Result<T, Errno> {
    match scope_for(at.dir) {
        Some(scope) => scope.run(op, &[at.name], scoped),
        None => system(),
    }
}

/// Runs `read` as `op` on `at`, or on its alias where the scope finds one.
pub fn resolved_at<T>(
    at: Entry<'_>,
    op: Op,
    read: impl Fn(Entry<'_>) -> Result<T, Errno>,
) -> Result<T, Errno> {
    match scope_for(at.dir) {
        Some(scope) => scope.run(op, &[at.name], |scope| scope.resolved(at, &read)),
        None => read(at),
    }
}

/// The most of `len` bytes one read or write moves.
pub fn io_limit(scope: Option<&Scope>, len: usize) -> usize {
    scope
        .and_then(|scope| scope.quirk(|quirks| quirks.short_io))
        .unwrap_or(len)
        .min(len)
}
