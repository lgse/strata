// SPDX-License-Identifier: MIT

use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Component, Components, Path, PathBuf},
};

use serde::Serialize;

use crate::model::{SortDirection, SortKey, rebase_path};

use super::super::icons_cell::{MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE};

mod keys;
pub(in crate::ui) use keys::key_for_location;
#[cfg(test)]
pub(in crate::ui) use keys::key_for_path;

pub(in crate::ui) const FOLDER_VIEWS_LIMIT: usize = 5_000;
const VERSION: i64 = 1;
/// TOML integers are signed, so use times stop here rather than fail to save.
const MAX_USED: u64 = i64::MAX as u64;

/// A local folder by absolute path, or a folder on a removable drive by its
/// filesystem UUID and path inside the drive, so it survives remounting.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::ui) enum FolderKey {
    Path(PathBuf),
    Volume { uuid: String, relative: PathBuf },
}

impl FolderKey {
    pub(in crate::ui) fn local(path: &Path) -> Option<Self> {
        local_path(path).map(Self::Path)
    }

    pub(in crate::ui) fn on_volume(uuid: &str, relative: &Path) -> Option<Self> {
        let uuid = uuid.trim();
        (!uuid.is_empty() && plain_names(relative.components()))
            .then(|| Self::with_path(Some(uuid), relative.components().collect()))
    }

    fn with_path(uuid: Option<&str>, path: PathBuf) -> Self {
        match uuid {
            None => Self::Path(path),
            Some(uuid) => Self::Volume {
                uuid: uuid.to_owned(),
                relative: path,
            },
        }
    }

    fn parts(&self) -> (Option<&str>, &Path) {
        match self {
            Self::Path(path) => (None, path),
            Self::Volume { uuid, relative } => (Some(uuid), relative),
        }
    }

    /// Whether `other` is this folder or inside it. Keys order by path
    /// components, so the folders inside one sort right after it.
    fn contains(&self, other: &Self) -> bool {
        let (uuid, path) = self.parts();
        let (other_uuid, other_path) = other.parts();
        uuid == other_uuid && other_path.starts_with(path)
    }

    fn rebased(&self, from: &Self, to: &Self) -> Option<Self> {
        let (uuid, path) = self.parts();
        let (from_uuid, from) = from.parts();
        if uuid != from_uuid {
            return None;
        }
        let (to_uuid, to) = to.parts();
        Some(Self::with_path(to_uuid, rebase_path(path, from, to)?))
    }
}

/// TOML keeps paths as strings, so only UTF-8 paths of plain names are remembered.
fn plain_names(mut components: Components<'_>) -> bool {
    components.as_path().to_str().is_some()
        && components.all(|component| matches!(component, Component::Normal(_)))
}

fn local_path(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    (components.next() == Some(Component::RootDir) && plain_names(components))
        .then(|| path.components().collect())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::ui) struct FolderView {
    pub(in crate::ui) sort: Option<(SortKey, SortDirection)>,
    pub(in crate::ui) icons_size: Option<i32>,
}

impl FolderView {
    fn is_empty(&self) -> bool {
        self.sort.is_none() && self.icons_size.is_none()
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    view: FolderView,
    used: u64,
}

/// Values taken out with a trashed folder, to put back if it is restored.
pub(in crate::ui) struct TakenViews(Vec<(FolderKey, Entry)>);

impl TakenViews {
    pub(in crate::ui) fn len(&self) -> usize {
        self.0.len()
    }
}

/// Per-folder values that differ from the defaults, with a logical use clock
/// for evicting the least recently used folders.
#[derive(Clone, Debug, Default)]
pub(in crate::ui) struct FolderViews {
    entries: BTreeMap<FolderKey, Entry>,
    clock: u64,
    /// Unsaved changes, kept so a save can merge them over what another Strata
    /// process (such as the portal chooser) wrote in the meantime.
    changed: HashMap<FolderKey, u64>,
    touched: HashMap<FolderKey, u64>,
    cleared: Option<u64>,
    edits: u64,
}

impl FolderViews {
    pub(in crate::ui) fn view(&self, key: &FolderKey) -> FolderView {
        self.entries
            .get(key)
            .map(|entry| entry.view)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(in crate::ui) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(in crate::ui) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(in crate::ui) fn has_within(&self, root: &FolderKey) -> bool {
        self.entries
            .range(root..)
            .next()
            .is_some_and(|(key, _)| root.contains(key))
    }

    pub(in crate::ui) fn may_forget(&self, root: &FolderKey) -> bool {
        let drive_root =
            matches!(root, FolderKey::Volume { relative, .. } if relative.as_os_str().is_empty());
        !drive_root && self.has_within(root)
    }

    fn keys_within(&self, root: &FolderKey) -> Vec<FolderKey> {
        self.entries
            .range(root..)
            .map(|(key, _)| key)
            .take_while(|key| root.contains(key))
            .cloned()
            .collect()
    }

    fn tick(&mut self) -> u64 {
        self.clock = (self.clock + 1).min(MAX_USED);
        self.clock
    }

    fn note_changed(&mut self, key: FolderKey) {
        self.edits += 1;
        self.changed.insert(key, self.edits);
    }

    pub(in crate::ui) fn edits(&self) -> u64 {
        self.edits
    }

    pub(in crate::ui) fn touch(&mut self, key: &FolderKey) -> bool {
        if !self.entries.contains_key(key) {
            return false;
        }
        let used = self.tick();
        if let Some(entry) = self.entries.get_mut(key) {
            entry.used = used;
        }
        self.edits += 1;
        self.touched.insert(key.clone(), self.edits);
        true
    }

    pub(in crate::ui) fn update(
        &mut self,
        key: &FolderKey,
        change: impl FnOnce(&mut FolderView),
    ) -> bool {
        let previous = self.view(key);
        let mut view = previous;
        change(&mut view);
        if view == previous {
            return false;
        }
        self.note_changed(key.clone());
        if view.is_empty() {
            self.entries.remove(key);
        } else {
            let used = self.tick();
            self.entries.insert(key.clone(), Entry { view, used });
            self.evict_to(FOLDER_VIEWS_LIMIT);
        }
        true
    }

    pub(in crate::ui) fn prune_default_sort(&mut self, sort: (SortKey, SortDirection)) -> bool {
        self.update_all(|view| {
            if view.sort == Some(sort) {
                view.sort = None;
            }
        })
    }

    pub(in crate::ui) fn prune_default_icons_size(&mut self, size: i32) -> bool {
        self.update_all(|view| {
            if view.icons_size == Some(size) {
                view.icons_size = None;
            }
        })
    }

    fn update_all(&mut self, mut change: impl FnMut(&mut FolderView)) -> bool {
        let mut changed = Vec::new();
        self.entries.retain(|key, entry| {
            let previous = entry.view;
            change(&mut entry.view);
            if entry.view != previous {
                changed.push(key.clone());
            }
            !entry.view.is_empty()
        });
        let any = !changed.is_empty();
        for key in changed {
            self.note_changed(key);
        }
        any
    }

    pub(in crate::ui) fn clear(&mut self) -> bool {
        let changed = !self.entries.is_empty();
        self.entries.clear();
        self.changed.clear();
        self.touched.clear();
        self.edits += 1;
        self.cleared = Some(self.edits);
        changed
    }

    /// Moves the values of `from` and the folders inside it to `to`, or forgets
    /// them for `None`. Like folder colors, a move drops stale values at and
    /// under its destination, while a merge into an existing folder keeps that
    /// folder's own values and carries the ones inside `from` over its contents.
    pub(in crate::ui) fn relocate(
        &mut self,
        from: &FolderKey,
        to: Option<&FolderKey>,
        merged: bool,
    ) -> bool {
        if to == Some(from) {
            return false;
        }
        let moving = self.keys_within(from);
        if moving.is_empty() {
            return false;
        }
        if let Some(to) = to.filter(|_| !merged) {
            for stale in self.keys_within(to) {
                if !from.contains(&stale) {
                    self.entries.remove(&stale);
                    self.note_changed(stale);
                }
            }
        }
        let mut moved = Vec::with_capacity(moving.len());
        for old in moving {
            let Some(entry) = self.entries.remove(&old) else {
                continue;
            };
            let new = to
                .filter(|_| !(merged && old == *from))
                .and_then(|to| old.rebased(from, to));
            self.note_changed(old);
            moved.extend(new.map(|new| (new, entry)));
        }
        for (key, _) in &moved {
            self.note_changed(key.clone());
        }
        self.entries.extend(moved);
        true
    }

    pub(in crate::ui) fn take_within(&mut self, root: &FolderKey) -> TakenViews {
        let taken = self
            .keys_within(root)
            .into_iter()
            .filter_map(|key| {
                let entry = self.entries.remove(&key)?;
                self.note_changed(key.clone());
                Some((key, entry))
            })
            .collect();
        TakenViews(taken)
    }

    /// A restore counts as the latest use, so restoring cannot evict what it restored.
    pub(in crate::ui) fn put_back(&mut self, mut taken: TakenViews) -> bool {
        if taken.0.is_empty() {
            return false;
        }
        taken.0.sort_unstable_by_key(|(_, entry)| entry.used);
        for (key, mut entry) in taken.0 {
            entry.used = self.tick();
            self.note_changed(key.clone());
            self.entries.insert(key, entry);
        }
        self.evict_to(FOLDER_VIEWS_LIMIT);
        true
    }

    /// `saved`, as another process may have left it, with this store's unsaved
    /// changes applied on top. Folders changed or opened here become the latest
    /// used, since the other process's clock may have run ahead of this one's,
    /// and the changes stay unsaved until a write succeeds.
    pub(in crate::ui) fn merged_over(&self, saved: Self) -> Self {
        let mut merged = if self.cleared.is_some() {
            Self::default()
        } else {
            saved
        };
        for key in self.changed.keys() {
            match self.entries.get(key) {
                Some(entry) => {
                    merged.entries.insert(key.clone(), *entry);
                }
                None => {
                    merged.entries.remove(key);
                }
            }
        }
        merged.clock = merged.clock.max(self.clock);
        let mut used_here: Vec<(u64, &FolderKey)> = self
            .changed
            .keys()
            .chain(
                self.touched
                    .keys()
                    .filter(|key| !self.changed.contains_key(*key)),
            )
            .filter_map(|key| Some((self.entries.get(key)?.used, key)))
            .collect();
        used_here.sort_unstable();
        for (_, key) in used_here {
            let used = merged.tick();
            if let Some(entry) = merged.entries.get_mut(key) {
                entry.used = used;
            }
        }
        merged.changed.clone_from(&self.changed);
        merged.touched.clone_from(&self.touched);
        merged.cleared = self.cleared;
        merged.edits = merged.edits.max(self.edits);
        merged.evict_to(FOLDER_VIEWS_LIMIT);
        merged
    }

    pub(in crate::ui) fn has_unsaved(&self) -> bool {
        self.cleared.is_some() || !self.changed.is_empty() || !self.touched.is_empty()
    }

    pub(in crate::ui) fn mark_saved_through(&mut self, edits: u64) {
        self.changed.retain(|_, edit| *edit > edits);
        self.touched.retain(|_, edit| *edit > edits);
        self.cleared = self.cleared.filter(|edit| *edit > edits);
    }

    pub(in crate::ui) fn same_views(&self, other: &Self) -> bool {
        self.entries.len() == other.entries.len()
            && self.entries.iter().all(|(key, entry)| {
                other
                    .entries
                    .get(key)
                    .is_some_and(|other| other.view == entry.view)
            })
    }

    fn evict_to(&mut self, limit: usize) {
        if self.entries.len() <= limit {
            return;
        }
        let mut by_use: Vec<(u64, FolderKey)> = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.used, key.clone()))
            .collect();
        by_use.sort_unstable();
        for (_, key) in by_use.into_iter().take(self.entries.len() - limit) {
            self.entries.remove(&key);
            self.note_changed(key);
        }
    }

    /// Fails only for invalid TOML or an unknown version, which saving must not
    /// overwrite. Invalid entries and fields are skipped individually, and the
    /// returned flag says whether anything was dropped or repaired.
    pub(in crate::ui) fn parse(contents: &str) -> io::Result<(Self, bool)> {
        let table: toml::Table = toml::from_str(contents)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        match table.get("version") {
            None => {}
            Some(toml::Value::Integer(version)) if *version == VERSION => {}
            Some(version) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported folder settings version {version}"),
                ));
            }
        }
        let mut repaired = table.keys().any(|key| key != "version" && key != "folder");
        let folders = match table.get("folder") {
            None => &[][..],
            Some(toml::Value::Array(folders)) => folders.as_slice(),
            Some(_) => {
                repaired = true;
                &[][..]
            }
        };
        let mut views = Self::default();
        for value in folders {
            let Some((key, entry, clean)) = parse_entry(value) else {
                repaired = true;
                continue;
            };
            repaired |= !clean;
            views.clock = views.clock.max(entry.used);
            match views.entries.get(&key) {
                Some(existing) => {
                    repaired = true;
                    if existing.used < entry.used {
                        views.entries.insert(key, entry);
                    }
                }
                None => {
                    views.entries.insert(key, entry);
                }
            }
        }
        if views.entries.len() > FOLDER_VIEWS_LIMIT {
            repaired = true;
            views.evict_to(FOLDER_VIEWS_LIMIT);
        }
        Ok((views, repaired))
    }

    pub(in crate::ui) fn to_toml(&self) -> io::Result<String> {
        let mut entries: Vec<(&FolderKey, &Entry)> = self.entries.iter().collect();
        entries.sort_by(|(left_key, left), (right_key, right)| {
            right
                .used
                .cmp(&left.used)
                .then_with(|| left_key.cmp(right_key))
        });
        let folder = entries
            .into_iter()
            .filter_map(|(key, entry)| {
                let (volume, path) = match key {
                    FolderKey::Path(path) => (None, path.to_str()?),
                    FolderKey::Volume { uuid, relative } => {
                        (Some(uuid.as_str()), relative.to_str()?)
                    }
                };
                let sort = entry
                    .view
                    .sort
                    .and_then(|(key, direction)| Some((key.stored_name()?, direction)));
                Some(StoredFolder {
                    volume,
                    path,
                    sort: sort.map(|(key, _)| key),
                    direction: sort.map(|(_, direction)| direction.stored_name()),
                    icons_size: entry.view.icons_size,
                    used: entry.used,
                })
            })
            .collect();
        toml::to_string_pretty(&StoredFile {
            version: VERSION,
            folder,
        })
        .map_err(io::Error::other)
    }
}

#[derive(Serialize)]
struct StoredFile<'a> {
    version: i64,
    folder: Vec<StoredFolder<'a>>,
}

#[derive(Serialize)]
struct StoredFolder<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<&'a str>,
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    direction: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icons_size: Option<i32>,
    used: u64,
}

/// The entry and whether it loaded without repairs.
fn parse_entry(value: &toml::Value) -> Option<(FolderKey, Entry, bool)> {
    let table = value.as_table()?;
    let path = Path::new(table.get("path")?.as_str()?);
    let key = match table.get("volume") {
        None => FolderKey::local(path)?,
        Some(volume) => FolderKey::on_volume(volume.as_str()?, path)?,
    };
    let mut clean = true;
    let sort = match (table.get("sort"), table.get("direction")) {
        (None, None) => None,
        (sort, direction) => {
            let sort = sort
                .and_then(toml::Value::as_str)
                .and_then(SortKey::from_stored)
                .zip(
                    direction
                        .and_then(toml::Value::as_str)
                        .and_then(SortDirection::from_stored),
                );
            clean &= sort.is_some();
            sort
        }
    };
    let icons_size = match table.get("icons_size") {
        None => None,
        Some(toml::Value::Integer(size)) => {
            let clamped = (*size).clamp(
                i64::from(MIN_ICONS_THUMBNAIL_SIZE),
                i64::from(MAX_ICONS_THUMBNAIL_SIZE),
            );
            clean &= clamped == *size;
            i32::try_from(clamped).ok()
        }
        Some(_) => {
            clean = false;
            None
        }
    };
    let used = match table.get("used") {
        None => 0,
        Some(toml::Value::Integer(used)) if *used >= 0 => *used as u64,
        Some(_) => {
            clean = false;
            0
        }
    };
    let view = FolderView { sort, icons_size };
    if view.is_empty() {
        return None;
    }
    Some((key, Entry { view, used }, clean))
}

#[cfg(test)]
mod tests;
