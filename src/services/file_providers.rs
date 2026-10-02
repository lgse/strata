// SPDX-License-Identifier: MIT
//! Opt-in external file providers. No provider code runs on the GTK thread.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    os::unix::{fs::MetadataExt, net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

const FRAME_LIMIT: usize = 1024 * 1024;
pub(crate) const PATH_LIMIT: usize = 200;
const DEADLINE: Duration = Duration::from_secs(8);

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub version: u32,
    pub id: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub icons: BTreeMap<String, String>,
}
#[derive(Clone)]
pub(crate) struct Registration {
    pub manifest: Manifest,
    pub icons: BTreeMap<String, Vec<u8>>,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Request {
    pub version: u32,
    pub id: u64,
    pub method: String,
    pub paths: Vec<String>,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Decoration {
    pub path: String,
    pub badge: Option<String>,
    #[serde(default)]
    pub description: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct MenuAction {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Default)]
pub(crate) struct Reply {
    pub version: u32,
    pub id: Option<u64>,
    pub event: Option<String>,
    #[serde(default)]
    pub decorations: Vec<Decoration>,
    #[serde(default)]
    pub actions: Vec<MenuAction>,
    #[serde(default)]
    pub message: String,
}
pub(crate) enum Update {
    Reply(Reply),
    Offline,
}
pub(crate) struct Client {
    pub requests: SyncSender<Request>,
    pub updates: Receiver<Update>,
}

fn trusted(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| {
        !m.file_type().is_symlink()
            && m.is_dir() == directory
            && (directory || m.is_file())
            && (m.uid() == rustix::process::getuid().as_raw() || m.uid() == 0)
            && m.mode() & 0o022 == 0
    })
}
fn slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn load(dir: &Path) -> Option<Registration> {
    if !trusted(dir, true) || !trusted(&dir.join("provider.json"), false) {
        return None;
    }
    let data = read_limited(&dir.join("provider.json"), 16384)?;
    let manifest: Manifest = serde_json::from_slice(&data).ok()?;
    if manifest.version != 1
        || !slug(&manifest.id)
        || dir.file_name()?.to_str()? != manifest.id
        || manifest.command.is_empty()
        || manifest.command.len() > 16
        || manifest.icons.len() > 16
        || manifest
            .command
            .iter()
            .any(|s| s.len() > 4096 || s.contains('\0'))
    {
        return None;
    }
    let exe = Path::new(&manifest.command[0]);
    // Resolve installed executable symlinks, never PATH or a browsed directory.
    let exe = exe
        .is_absolute()
        .then(|| fs::canonicalize(exe).ok())
        .flatten()?;
    if !trusted(&exe, false) {
        return None;
    }
    let mut icons = BTreeMap::new();
    for (id, filename) in &manifest.icons {
        if !slug(id) || Path::new(filename).components().count() != 1 || !filename.ends_with(".png")
        {
            return None;
        }
        let path = dir.join(filename);
        if !trusted(&path, false) {
            return None;
        }
        let bytes = read_limited(&path, 65536)?;
        if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return None;
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        icons.insert(id.clone(), bytes);
    }
    Some(Registration { manifest, icons })
}
fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit as usize).then_some(bytes)
}
pub(crate) fn discover(root: &Path) -> Vec<Registration> {
    if !trusted(root, true) {
        return Vec::new();
    }
    let Ok(dirs) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = dirs
        .take(64)
        .filter_map(Result::ok)
        .map(|d| d.path())
        .collect();
    dirs.sort();
    dirs.iter().filter_map(|p| load(p)).take(8).collect()
}
pub(crate) fn config_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))?;
    Some(base.join("strata/providers"))
}
impl Request {
    fn valid(&self) -> bool {
        self.version == 1
            && self.paths.len() <= PATH_LIMIT
            && !self.paths.is_empty()
            && self
                .paths
                .iter()
                .all(|p| p.len() <= 16384 && Path::new(p).is_absolute() && !p.contains('\0'))
            && matches!(self.method.as_str(), "query" | "menu" | "activate")
            && self.action.as_ref().is_none_or(|a| slug(a))
    }
}
fn checked_reply(bytes: &[u8], manifest: &Manifest) -> io::Result<Reply> {
    let r: Reply = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if r.version != 1
        || (r.id.is_some() == r.event.is_some())
        || r.decorations.len() > PATH_LIMIT
        || r.actions.len() > 16
        || r.message.len() > 16384
        || r.decorations.iter().any(|d| {
            d.path.len() > 16384
                || d.description.len() > 512
                || d.badge
                    .as_ref()
                    .is_some_and(|i| !manifest.icons.contains_key(i))
        })
        || r.actions.iter().any(|a| {
            !slug(&a.id)
                || a.label.is_empty()
                || a.label.len() > 128
                || a.label.chars().any(char::is_control)
                || a.icon
                    .as_ref()
                    .is_some_and(|i| !manifest.icons.contains_key(i))
        })
        || (r.event.is_some() && (r.event.as_deref() != Some("invalidate") || r.id.is_some()))
    {
        return Err(io::Error::other("invalid provider response"));
    }
    Ok(r)
}
pub(crate) fn start(registration: Registration) -> Client {
    let (tx, rx) = mpsc::sync_channel(16);
    let (updates, out) = mpsc::sync_channel(32);
    thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            if run(&registration.manifest, &rx, &updates, first).is_err() {
                thread::sleep(Duration::from_secs(2));
                // Drop queued actions too: an uncertain action is never replayed.
                while rx.try_recv().is_ok() {}
                if updates.try_send(Update::Offline).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    });
    Client {
        requests: tx,
        updates: out,
    }
}
fn run(
    manifest: &Manifest,
    requests: &Receiver<Request>,
    updates: &SyncSender<Update>,
    first: Request,
) -> io::Result<()> {
    let (mut stream, peer) = UnixStream::pair()?;
    stream.set_read_timeout(Some(Duration::from_millis(25)))?;
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    let mut child = Command::new(&manifest.command[0])
        .args(&manifest.command[1..])
        .current_dir("/")
        .process_group(0)
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(peer.try_clone()?)))
        .stdout(Stdio::from(std::os::fd::OwnedFd::from(peer)))
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        let mut buffer = Vec::new();
        let mut pending = BTreeMap::new();
        let mut next = Some(first);
        loop {
            if let Some(r) = next.take() {
                if !r.valid() {
                    return Err(io::Error::other("invalid request"));
                }
                let mut line = serde_json::to_vec(&r)?;
                line.push(b'\n');
                if line.len() > FRAME_LIMIT || pending.len() >= 16 {
                    return Err(io::Error::other("provider busy"));
                }
                stream.write_all(&line)?;
                pending.insert(r.id, Instant::now());
            }
            if pending.values().any(|at| at.elapsed() > DEADLINE) {
                return Err(io::Error::other("provider timeout"));
            }
            let mut bytes = [0; 8192];
            match stream.read(&mut bytes) {
                Ok(0) => return Err(io::Error::other("provider closed")),
                Ok(n) => buffer.extend_from_slice(&bytes[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e),
            }
            if buffer.len() > FRAME_LIMIT {
                return Err(io::Error::other("provider frame limit"));
            }
            while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                let reply = checked_reply(&buffer[..end], manifest)?;
                buffer.drain(..=end);
                if let Some(id) = reply.id
                    && pending.remove(&id).is_none()
                {
                    continue;
                }
                updates
                    .try_send(Update::Reply(reply))
                    .map_err(io::Error::other)?;
            }
            next = match requests.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return Ok(()),
            };
        }
    })();
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}
#[cfg(test)]
mod tests;
