// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use gtk::{gio, glib};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GitStatus {
    Modified,
    Untracked,
    Ignored,
}

impl GitStatus {
    pub fn badge_text(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Untracked => "U",
            Self::Ignored => "I",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Modified => "Modified",
            Self::Untracked => "Untracked",
            Self::Ignored => "Ignored",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Modified => "git-badge-modified",
            Self::Untracked => "git-badge-untracked",
            Self::Ignored => "git-badge-ignored",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitHead {
    Branch(String),
    Detached(String),
}

impl GitHead {
    pub fn display_text(&self) -> String {
        match self {
            Self::Branch(branch) => branch.clone(),
            Self::Detached(commit) => format!("Detached · {commit}"),
        }
    }

    pub fn accessible_label(&self) -> String {
        match self {
            Self::Branch(branch) => format!("Current Git branch: {branch}"),
            Self::Detached(commit) => format!("Detached Git HEAD at commit {commit}"),
        }
    }

    pub fn is_detached(&self) -> bool {
        matches!(self, Self::Detached(_))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ParsedGitStatus {
    status_map: HashMap<PathBuf, GitStatus>,
    collapsed_dirs: Vec<(PathBuf, GitStatus)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepoGitStatus {
    root: PathBuf,
    head: Option<GitHead>,
    status_map: HashMap<PathBuf, GitStatus>,
    collapsed_dirs: Vec<(PathBuf, GitStatus)>,
}

impl RepoGitStatus {
    fn status_for_child(&self, relative_path: &Path, is_directory: bool) -> Option<GitStatus> {
        if let Some(&status) = self.status_map.get(relative_path) {
            return Some(status);
        }

        if let Some((_, status)) = self
            .collapsed_dirs
            .iter()
            .filter(|(dir, _)| relative_path.starts_with(dir))
            .max_by_key(|(dir, status)| {
                (
                    dir.components().count(),
                    std::cmp::Reverse(status_priority(*status)),
                )
            })
        {
            return Some(*status);
        }

        is_directory
            .then(|| {
                self.status_map
                    .iter()
                    .filter(|(path, _)| path.starts_with(relative_path))
                    .map(|(_, status)| *status)
                    .min_by_key(|status| status_priority(*status))
            })
            .flatten()
    }
}

fn status_priority(status: GitStatus) -> u8 {
    match status {
        GitStatus::Modified => 0,
        GitStatus::Untracked => 1,
        GitStatus::Ignored => 2,
    }
}

fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() { path } else { path.parent()? };
    loop {
        if current.join(".git").symlink_metadata().is_ok() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn find_status_repo_root(path: &Path, is_directory: bool) -> Option<PathBuf> {
    let discovery_path = if is_directory { path.parent()? } else { path };
    find_repo_root(discovery_path)
}

fn parse_porcelain_v1_z(output: &[u8]) -> ParsedGitStatus {
    let mut parsed = ParsedGitStatus::default();
    let mut iter = output.split(|&byte| byte == 0);

    while let Some(chunk) = iter.next() {
        if chunk.len() < 4 || chunk[2] != b' ' {
            continue;
        }
        let x = chunk[0];
        let y = chunk[1];
        let raw_path = &chunk[3..];
        if x == b'R' || x == b'C' {
            let _old_path = iter.next();
        }
        let (clean_path, collapsed) = raw_path
            .strip_suffix(b"/")
            .map_or((raw_path, false), |path| (path, true));
        if clean_path.is_empty() {
            continue;
        }
        let path = PathBuf::from(OsString::from_vec(clean_path.to_vec()));
        let status = match (x, y) {
            (b'?', b'?') => GitStatus::Untracked,
            (b'!', b'!') => GitStatus::Ignored,
            _ => GitStatus::Modified,
        };
        if collapsed && matches!(status, GitStatus::Untracked | GitStatus::Ignored) {
            parsed.collapsed_dirs.push((path.clone(), status));
        }
        parsed.status_map.insert(path, status);
    }

    parsed
}

fn git_command(repo_root: &Path) -> Option<Command> {
    let mut command = crate::trusted_command::command("git").ok()?;
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .arg("--no-pager")
        .args(["-c", "core.fsmonitor=false"])
        .arg("-C")
        .arg(repo_root);
    Some(command)
}

fn git_output(repo_root: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let output = git_command(repo_root)?.args(arguments).output().ok()?;
    output.status.success().then_some(output.stdout)
}

fn query_git_head(repo_root: &Path) -> Option<GitHead> {
    if let Some(output) = git_output(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        let branch = String::from_utf8_lossy(&output).trim().to_owned();
        if !branch.is_empty() {
            return Some(GitHead::Branch(branch));
        }
    }

    let output = git_output(repo_root, &["rev-parse", "--short=8", "--verify", "HEAD"])?;
    let commit = String::from_utf8_lossy(&output).trim().to_owned();
    (!commit.is_empty()).then_some(GitHead::Detached(commit))
}

fn query_git_status(repo_root: &Path) -> Option<RepoGitStatus> {
    let output = git_output(
        repo_root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--ignored=matching",
        ],
    )
    .or_else(|| {
        git_output(
            repo_root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=normal",
                "--ignored",
            ],
        )
    })?;

    let parsed = parse_porcelain_v1_z(&output);
    Some(RepoGitStatus {
        root: repo_root.to_path_buf(),
        head: query_git_head(repo_root),
        status_map: parsed.status_map,
        collapsed_dirs: parsed.collapsed_dirs,
    })
}

const CACHE_TTL: Duration = Duration::from_secs(2);

struct CacheEntry {
    status: Option<RepoGitStatus>,
    timestamp: Instant,
}

type GitStatusListener = Box<dyn Fn(&Path) + Send + Sync>;

pub struct GitService {
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
    in_flight: Mutex<HashSet<PathBuf>>,
    listeners: Mutex<Vec<GitStatusListener>>,
}

impl GitService {
    fn global() -> &'static Self {
        static SERVICE: OnceLock<GitService> = OnceLock::new();
        SERVICE.get_or_init(|| GitService {
            cache: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            listeners: Mutex::new(Vec::new()),
        })
    }

    pub fn add_listener(listener: impl Fn(&Path) + Send + Sync + 'static) {
        if let Ok(mut listeners) = Self::global().listeners.lock() {
            listeners.push(Box::new(listener));
        }
    }

    pub fn status_for_path(path: &Path, is_directory: bool) -> Option<GitStatus> {
        let root = find_status_repo_root(path, is_directory)?;
        let relative_path = path.strip_prefix(&root).ok()?;
        Self::cached_value(&root, |status| {
            status.status_for_child(relative_path, is_directory)
        })
        .flatten()
    }

    pub fn head_for_path(path: &Path) -> Option<GitHead> {
        let root = find_repo_root(path)?;
        Self::cached_value(&root, |status| status.head.clone()).flatten()
    }

    pub fn refresh_path(path: &Path) {
        let Some(root) = find_repo_root(path) else {
            return;
        };
        if let Ok(mut cache) = Self::global().cache.lock()
            && let Some(entry) = cache.get_mut(&root)
        {
            entry.timestamp = Instant::now()
                .checked_sub(CACHE_TTL)
                .unwrap_or_else(Instant::now);
        }
        Self::schedule_query(&root);
    }

    fn cached_value<T>(root: &Path, read: impl FnOnce(&RepoGitStatus) -> T) -> Option<T> {
        let (value, fresh) = {
            let cache = Self::global().cache.lock().ok()?;
            match cache.get(root) {
                Some(entry) => (
                    entry.status.as_ref().map(read),
                    entry.timestamp.elapsed() < CACHE_TTL,
                ),
                None => (None, false),
            }
        };
        if !fresh {
            Self::schedule_query(root);
        }
        value
    }

    fn schedule_query(root: &Path) {
        {
            let mut in_flight = match Self::global().in_flight.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if !in_flight.insert(root.to_path_buf()) {
                return;
            }
        }

        let root_for_task = root.to_path_buf();
        gio::spawn_blocking(move || {
            let status = query_git_status(&root_for_task);
            if let Ok(mut cache) = Self::global().cache.lock() {
                cache.insert(
                    root_for_task.clone(),
                    CacheEntry {
                        status,
                        timestamp: Instant::now(),
                    },
                );
            }
            if let Ok(mut in_flight) = Self::global().in_flight.lock() {
                in_flight.remove(&root_for_task);
            }
            let root_for_notify = root_for_task.clone();
            glib::idle_add_once(move || {
                if let Ok(listeners) = Self::global().listeners.lock() {
                    for listener in listeners.iter() {
                        listener(&root_for_notify);
                    }
                }
            });
        });
    }
}
