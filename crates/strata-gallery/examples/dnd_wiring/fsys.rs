// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use strata_components::contract::dnd::{VolumeRelation, file_uri, uri_to_path};
use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource};

/// The next entry id, for every [`Ids`] in the process.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct Ids {
    by_path: HashMap<PathBuf, EntryId>,
    by_id: HashMap<EntryId, PathBuf>,
}

impl Ids {
    pub fn of(&mut self, path: &Path) -> EntryId {
        if let Some(id) = self.by_path.get(path) {
            return *id;
        }
        let id = EntryId::new(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        self.by_path.insert(path.to_owned(), id);
        self.by_id.insert(id, path.to_owned());
        id
    }

    pub fn path(&self, id: EntryId) -> Option<&Path> {
        self.by_id.get(&id).map(PathBuf::as_path)
    }

    pub fn uri(&self, id: EntryId) -> Option<String> {
        self.path(id).map(file_uri)
    }
}

/// `folder`'s entries, folders first then by name (case folded), hidden ones left out.
pub fn list(ids: &mut Ids, folder: &Path) -> ListSource {
    let mut entries: Vec<(bool, String, PathBuf)> = std::fs::read_dir(folder)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        return None;
                    }
                    let dir = e.file_type().is_ok_and(|t| t.is_dir());
                    Some((dir, name, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    ListSource::new(
        entries
            .into_iter()
            .map(|(dir, name, path)| {
                let (kind, icon) = if dir {
                    (EntryKind::Folder, "folder")
                } else {
                    (EntryKind::File, "file")
                };
                ListEntry::new(ids.of(&path), name, kind, icon)
            })
            .collect(),
    )
}

/// Paths from a drop's URIs, local files only.
pub fn paths(uris: &[String]) -> Vec<PathBuf> {
    uris.iter().filter_map(|u| uri_to_path(u)).collect()
}

/// The demo's volumes: everything under `<root>/usb` is another device.
pub fn volume(root: &Path, sources: &[PathBuf], dest: &Path) -> VolumeRelation {
    if sources.is_empty() {
        return VolumeRelation::Unknown;
    }
    let usb = root.join("usb");
    let on_usb = |p: &Path| p.starts_with(&usb);
    if sources.iter().all(|s| on_usb(s) == on_usb(dest)) {
        VolumeRelation::Same
    } else {
        VolumeRelation::Different
    }
}

pub fn no_op(sources: &[PathBuf], dest: &Path) -> bool {
    !sources.is_empty()
        && sources
            .iter()
            .all(|s| s.parent() == Some(dest) || dest.starts_with(s))
}

pub fn apply(
    root: &Path,
    sources: &[PathBuf],
    dest: &Path,
    moving: bool,
) -> Result<Vec<String>, String> {
    let root = root.canonicalize().map_err(|e| format!("root: {e}"))?;
    let dest = dest
        .canonicalize()
        .map_err(|e| format!("destination: {e}"))?;
    if !dest.starts_with(&root) || !dest.is_dir() {
        return Err(format!("{} is no folder inside the root", dest.display()));
    }
    let sources = sources
        .iter()
        .map(|s| inside(&root, s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut written = Vec::new();
    for src in sources {
        if dest.starts_with(&src) || src.parent() == Some(dest.as_path()) {
            continue;
        }
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let to = place(&dest, &name, |to| {
            if moving {
                move_all(&src, to)
            } else {
                copy_all(&src, to)
            }
        })
        .map_err(|e| format!("{}: {e}", src.display()))?;
        written.push(
            to.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    Ok(written)
}

fn inside(root: &Path, src: &Path) -> Result<PathBuf, String> {
    let outside = || format!("{} is outside the root", src.display());
    let name = src.file_name().ok_or_else(outside)?;
    let folder = src
        .parent()
        .ok_or_else(outside)?
        .canonicalize()
        .map_err(|e| format!("source: {e}"))?;
    let src = folder.join(name);
    src.symlink_metadata().map_err(|e| format!("source: {e}"))?;
    if !src.starts_with(root) || src == root {
        return Err(outside());
    }
    Ok(src)
}

fn place(
    dest: &Path,
    name: &str,
    mut make: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    for n in 0u32.. {
        let to = match n {
            0 => dest.join(name),
            n => dest.join(format!("{name} ({n})")),
        };
        match make(&to) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("no free name"))
}

fn below(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::AlreadyExists {
        io::Error::other(e)
    } else {
        e
    }
}

/// Copies `from` to `to`, which must not exist: a link as the link, a folder with its entries.
fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    let meta = from.symlink_metadata()?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
    } else if kind.is_dir() {
        std::fs::create_dir(to)?;
        for e in std::fs::read_dir(from).map_err(below)? {
            let e = e.map_err(below)?;
            copy_all(&e.path(), &to.join(e.file_name())).map_err(below)?;
        }
        std::fs::set_permissions(to, meta.permissions()).map_err(below)
    } else if kind.is_file() {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        let mut file = std::fs::File::open(from).map_err(below)?;
        io::copy(&mut file, &mut out).map_err(below)?;
        out.set_permissions(meta.permissions()).map_err(below)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "not a file, folder or link",
        ))
    }
}

fn move_all(from: &Path, to: &Path) -> io::Result<()> {
    let across = |e: &io::Error| e.kind() == io::ErrorKind::CrossesDevices;
    if from.symlink_metadata()?.file_type().is_dir() {
        std::fs::create_dir(to)?;
        return match std::fs::rename(from, to) {
            Ok(()) => Ok(()),
            Err(e) => {
                std::fs::remove_dir(to).map_err(below)?;
                if !across(&e) {
                    return Err(below(e));
                }
                copy_all(from, to)?;
                std::fs::remove_dir_all(from).map_err(below)
            }
        };
    }
    // A hard link fails if `to` exists (and links a link itself, not what it points at).
    match std::fs::hard_link(from, to) {
        Ok(()) => std::fs::remove_file(from).or_else(|e| {
            // The source stays (its folder is read-only, say): so must nothing else.
            std::fs::remove_file(to).map_err(below)?;
            Err(below(e))
        }),
        Err(e) if across(&e) => copy_then_remove(from, to),
        Err(e) => Err(e),
    }
}

fn copy_then_remove(from: &Path, to: &Path) -> io::Result<()> {
    copy_all(from, to)?;
    std::fs::remove_file(from).or_else(|e| {
        std::fs::remove_file(to).map_err(below)?;
        Err(below(e))
    })
}

#[cfg(test)]
mod tests;
