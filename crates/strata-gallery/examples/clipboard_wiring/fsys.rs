//! The demo's stand-in for Strata's file side: entry ids by path, listings read from disk, and
//! pastes applied inside `--root` only.
//!
//! The guards (`inside`, `place`, `copy_all`, `move_all`, `below`) are X2's
//! (`examples/dnd_wiring/fsys.rs`), copied while both demos are in review and adapted for a
//! paste: a copy into its own folder takes a numbered name, as Strata's Duplicate does, and a
//! name taken elsewhere is kept beside under a numbered name (Keep Both). A follow-up shares
//! them in `examples/common/transfer.rs` once both merge.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource};

/// Entry ids by path: an id lives for the process and is never reused (W1).
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
    /// The source the paste stopped at, and why: what came before it is done (and listed
    /// above), what came after it is untouched.
    pub failed: Option<(String, String)>,
}

/// Pastes `sources` into `dest`, copying or `moving`. Refuses the whole paste if `dest` or any
/// source is outside `root`, before touching anything. A copy into its own folder takes the
/// first free numbered name (`name (1).ext`); a move into its own folder, and a folder into
/// itself or one of its descendants, does nothing. A name taken in `dest` is kept beside
/// under the first free numbered name (Keep Both). A link is copied or moved as the link
/// (never followed out of the root), and nothing is created over an existing entry.
///
/// A source that fails stops the paste there, in [`Pasted::failed`]; the Ok still says what
/// was done before it, so a cut's moved sources still leave the clipboard.
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

/// Writes a pasted image into `dest` as `image.png`, else the first free `image (n).png`.
/// The name written.
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

/// `src` with its folder resolved but not itself (a pasted link is the link), if it is there
/// and inside `root`. (X2's.)
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

/// The names `name` may take, in order: itself when `plain`, then Strata's numbered names
/// (`stem (n).ext`, a file's last extension kept; one past the number it has already).
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

/// A stem's ` (n)` suffix, as Strata reads it (`parse_copy_suffix`): digits, no leading zero,
/// and below `u64::MAX`, so one past it is a number too.
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

/// Makes an entry in `dest` with `make`, under the first of [`names`] free: `make` creates it
/// exclusively, failing with `AlreadyExists` only when the name is taken. (X2's, with
/// Strata's numbering.)
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

/// An error below the entry `place` made: never "the name is taken", which would have it
/// try another name over a half-made entry. (X2's.)
fn below(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::AlreadyExists {
        io::Error::other(e)
    } else {
        e
    }
}

/// Copies `from` to `to`, which must not exist: a link as the link, a folder with its entries.
/// (X2's.)
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

/// Moves `from` to `to`, which must not exist, never replacing an entry there. Copies and
/// removes only across devices: any other failure is reported, with the source untouched.
/// A file whose source can't be removed once its new name is made loses the new name again,
/// so nothing is left in both places. (X2's, with its review-2 fix and that.)
fn move_all(from: &Path, to: &Path) -> io::Result<()> {
    if from.symlink_metadata()?.file_type().is_dir() {
        // Reserve the name with an empty folder of our own, which `rename` replaces (it only
        // replaces an empty folder).
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
