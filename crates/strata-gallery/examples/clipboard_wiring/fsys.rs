// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource};

#[derive(Default)]
pub struct Ids {
    next: u64,
    by_path: HashMap<PathBuf, EntryId>,
    by_id: HashMap<EntryId, PathBuf>,
}

impl Ids {
    pub fn of(&mut self, path: &Path) -> EntryId {
        if let Some(id) = self.by_path.get(path) {
            return *id;
        }
        self.next += 1;
        let id = EntryId::new(self.next);
        self.by_path.insert(path.to_owned(), id);
        self.by_id.insert(id, path.to_owned());
        id
    }

    pub fn path(&self, id: EntryId) -> Option<&Path> {
        self.by_id.get(&id).map(PathBuf::as_path)
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

/// What a paste did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pasted {
    /// The names written in the destination, in source order.
    pub written: Vec<String>,
    /// The sources a move took away (for the clipboard's cut list).
    pub moved: Vec<PathBuf>,
    /// Names taken in the destination, kept beside: (the name, the name written).
    pub conflicts: Vec<(String, String)>,
    /// Sources left alone, and why.
    pub skipped: Vec<(String, &'static str)>,
    pub failed: Option<(String, String)>,
}

pub fn paste(
    root: &Path,
    sources: &[PathBuf],
    dest: &Path,
    moving: bool,
) -> Result<Pasted, String> {
    let root = root.canonicalize().map_err(|e| format!("root: {e}"))?;
    let dest = folder_inside(&root, dest)?;
    let sources = sources
        .iter()
        .map(|s| inside(&root, s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut done = Pasted::default();
    for src in sources {
        let name = src.file_name().map(OsStr::to_owned).unwrap_or_default();
        let shown = name.to_string_lossy().into_owned();
        if dest.starts_with(&src) {
            done.skipped.push((shown, "into itself"));
            continue;
        }
        let own_folder = src.parent() == Some(dest.as_path());
        if own_folder && moving {
            done.skipped.push((shown, "its own folder"));
            continue;
        }
        let is_dir = src.symlink_metadata().is_ok_and(|m| m.is_dir());
        let placed = place(&dest, &name, is_dir, !own_folder, |to| {
            if moving {
                move_all(&src, to)
            } else {
                copy_all(&src, to)
            }
        });
        let to = match placed {
            Ok(to) => to,
            Err(e) => {
                done.failed = Some((shown, e.to_string()));
                break;
            }
        };
        let written = to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !own_folder && written != shown {
            done.conflicts.push((shown, written.clone()));
        }
        done.written.push(written);
        if moving {
            done.moved.push(src);
        }
    }
    Ok(done)
}

pub fn paste_image(root: &Path, dest: &Path, png: &[u8]) -> Result<String, String> {
    let root = root.canonicalize().map_err(|e| format!("root: {e}"))?;
    let dest = folder_inside(&root, dest)?;
    let to = place(&dest, OsStr::new("image.png"), false, true, |to| {
        use std::io::Write;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        out.write_all(png).map_err(below)
    })
    .map_err(|e| format!("image.png: {e}"))?;
    Ok(to
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// `dest` resolved, if it is a folder inside `root`.
fn folder_inside(root: &Path, dest: &Path) -> Result<PathBuf, String> {
    let dest = dest
        .canonicalize()
        .map_err(|e| format!("destination: {e}"))?;
    if !dest.starts_with(root) || !dest.is_dir() {
        return Err(format!("{} is no folder inside the root", dest.display()));
    }
    Ok(dest)
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

fn names(name: &OsStr, is_dir: bool, plain: bool) -> impl Iterator<Item = OsString> + '_ {
    let (stem, ext) = parts(name, is_dir);
    let (base, has) = number_of(stem);
    let start = has.map_or(1, |n| n + 1);
    plain
        .then(|| name.to_owned())
        .into_iter()
        .chain((start..=start.saturating_add(9_999)).map(move |n| {
            let mut out = base.as_bytes().to_vec();
            out.extend_from_slice(format!(" ({n})").as_bytes());
            out.extend_from_slice(ext.as_bytes());
            OsString::from_vec(out)
        }))
}

/// The stem and the last extension (with its dot) of a file's name; a folder's whole name.
fn parts(name: &OsStr, is_dir: bool) -> (&OsStr, &OsStr) {
    let path = Path::new(name);
    let stem = match path.extension() {
        Some(ext) if !is_dir && !ext.is_empty() => path.file_stem().unwrap_or(name),
        _ => name,
    };
    (stem, OsStr::from_bytes(&name.as_bytes()[stem.len()..]))
}

fn number_of(stem: &OsStr) -> (&OsStr, Option<u64>) {
    let bytes = stem.as_bytes();
    if let Some(open) = bytes.strip_suffix(b")")
        && let Some(at) = open.windows(2).rposition(|w| w == b" (")
    {
        let digits = &open[at + 2..];
        if !digits.is_empty()
            && digits[0] != b'0'
            && digits.iter().all(u8::is_ascii_digit)
            && let Some(n) = std::str::from_utf8(digits)
                .ok()
                .and_then(|d| d.parse::<u64>().ok())
            && n < u64::MAX
        {
            return (OsStr::from_bytes(&bytes[..at]), Some(n));
        }
    }
    (stem, None)
}

fn place(
    dest: &Path,
    name: &OsStr,
    is_dir: bool,
    plain: bool,
    mut make: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    for n in names(name, is_dir, plain) {
        let to = dest.join(n);
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
    if from.symlink_metadata()?.file_type().is_dir() {
        std::fs::create_dir(to)?;
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {}
            Err(e) => {
                let _ = std::fs::remove_dir(to);
                return Err(below(e));
            }
        }
        std::fs::remove_dir(to).map_err(below)?;
        copy_all(from, to)?;
        return std::fs::remove_dir_all(from).map_err(below);
    }
    // A hard link fails if `to` exists (and links a link itself, not what it points at).
    match std::fs::hard_link(from, to) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => copy_all(from, to)?,
        Err(e) => return Err(e),
    }
    std::fs::remove_file(from).map_err(|e| {
        // Only while the source is still there: else the new name is the only one left.
        if from.symlink_metadata().is_ok() {
            let _ = std::fs::remove_file(to);
        }
        below(e)
    })
}

#[cfg(test)]
mod tests;
