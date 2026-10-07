// SPDX-License-Identifier: MIT
//! Opt-in external file providers. No provider code runs on the GTK thread.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

mod protocol;
mod transport;

pub(crate) use protocol::{
    Decoration, MenuAction, OutcomeStatus, Reply, Request, query_batch, selection_fits,
};
pub(crate) use transport::{Client, Update, start};
pub(crate) const PATH_LIMIT: usize = 200;

#[derive(Clone, Deserialize)]
pub(crate) struct Manifest {
    pub version: u32,
    pub id: String,
    pub name: Option<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub icons: BTreeMap<String, String>,
}
#[derive(Clone)]
pub(crate) struct Registration {
    pub manifest: Manifest,
    pub icons: BTreeMap<String, Vec<u8>>,
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
        || manifest.name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 64 || name.chars().any(char::is_control)
        })
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
    dirs.iter()
        .filter_map(|path| {
            let loaded = load(path);
            if loaded.is_none() {
                let id = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|id| slug(id))
                    .unwrap_or("invalid-id");
                eprintln!(
                    "Strata provider {id}: registration rejected (permissions, manifest or artwork)"
                );
            }
            loaded
        })
        .take(8)
        .collect()
}
pub(crate) fn config_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))?;
    Some(base.join("strata/providers"))
}
#[cfg(test)]
mod tests;
