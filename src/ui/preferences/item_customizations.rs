// SPDX-License-Identifier: MIT

//! Folder colors and custom icons are keyed by absolute path, so the file
//! operations Strata performs carry, drop or bring back those keys.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::glib;

use crate::services::TrashedOriginal;

use super::{PreferenceManager, Preferences};

/// Trashed customization keys kept for Put back or undo; the oldest go first.
const TRASHED_KEYS_LIMIT: usize = 10_000;

#[derive(Default)]
pub(super) struct ItemCustomizationState {
    trashed: RefCell<TrashedCustomizations>,
    save_scheduled: Cell<bool>,
}

/// Customizations of trashed items, kept in memory for this session only.
#[derive(Default)]
struct TrashedCustomizations {
    items: VecDeque<TrashedItem>,
    keys: usize,
}

struct TrashedItem {
    root: PathBuf,
    identity: TrashedOriginal,
    folder_colors: Vec<(String, String)>,
    custom_icons: Vec<(String, String)>,
}

impl TrashedItem {
    fn keys(&self) -> usize {
        self.folder_colors.len() + self.custom_icons.len()
    }
}

impl TrashedCustomizations {
    /// Two live items never share an identity, so an older entry with this
    /// identity belongs to an item that has left Trash and whose inode was reused.
    fn forget_identity(&mut self, identity: TrashedOriginal) {
        let keys = &mut self.keys;
        self.items.retain(|item| {
            let keep = item.identity != identity;
            if !keep {
                *keys -= item.keys();
            }
            keep
        });
    }

    fn push(&mut self, item: TrashedItem) {
        self.keys += item.keys();
        self.items.push_back(item);
        while self.keys > TRASHED_KEYS_LIMIT && self.items.len() > 1 {
            if let Some(oldest) = self.items.pop_front() {
                self.keys -= oldest.keys();
            }
        }
    }

    fn take(&mut self, root: &Path, identity: TrashedOriginal) -> Option<TrashedItem> {
        let index = self
            .items
            .iter()
            .position(|item| item.root == root && item.identity == identity)?;
        let item = self.items.remove(index)?;
        self.keys -= item.keys();
        Some(item)
    }
}

impl PreferenceManager {
    /// Carries the customization of the item at `from`, and of everything under
    /// it, to `to`, or drops them when `to` is `None` because the item left
    /// local storage. Stale keys at and under a non-merged destination are
    /// dropped first; a merged destination keeps its own key.
    pub fn relocate_item_customizations(
        self: &Rc<Self>,
        from: &Path,
        to: Option<&Path>,
        merged_into_existing: bool,
    ) {
        let Some(to) = to else {
            self.forget_item_customizations(from, None);
            return;
        };
        let from = key_path(from);
        let to = key_path(to);
        if from == to {
            return;
        }
        let mut changed = false;
        {
            let mut preferences = self.preferences.borrow_mut();
            for map in item_maps(&mut preferences) {
                changed |= relocate(map, &from, &to, merged_into_existing);
            }
        }
        if changed {
            self.finish_item_customization_change(&[from, to]);
        }
    }

    /// Drops the customizations at and under `root`. A `trash_identity` keeps
    /// them for [`Self::restore_item_customizations`] until the session ends.
    pub fn forget_item_customizations(
        self: &Rc<Self>,
        root: &Path,
        trash_identity: Option<TrashedOriginal>,
    ) {
        let root = key_path(root);
        let changed = {
            let mut preferences = self.preferences.borrow_mut();
            let mut trashed = self.item_customizations.trashed.borrow_mut();
            if let Some(identity) = trash_identity {
                trashed.forget_identity(identity);
            }
            let folder_colors = take_within(&mut preferences.folder_colors, &root);
            let custom_icons = take_within(&mut preferences.custom_icons, &root);
            let changed = !folder_colors.is_empty() || !custom_icons.is_empty();
            if let Some(identity) = trash_identity.filter(|_| changed) {
                trashed.push(TrashedItem {
                    root: root.clone(),
                    identity,
                    folder_colors,
                    custom_icons,
                });
            }
            changed
        };
        if changed {
            self.finish_item_customization_change(&[root]);
        }
    }

    /// Re-applies what [`Self::forget_item_customizations`] kept for the very
    /// item now back at `root`; another item restored there gets nothing.
    pub fn restore_item_customizations(self: &Rc<Self>, root: &Path) {
        let root = key_path(root);
        let mut trashed = self.item_customizations.trashed.borrow_mut();
        if !trashed.items.iter().any(|item| item.root == root) {
            return;
        }
        let Some(item) =
            TrashedOriginal::at_path(&root).and_then(|identity| trashed.take(&root, identity))
        else {
            return;
        };
        drop(trashed);
        {
            let mut preferences = self.preferences.borrow_mut();
            preferences.folder_colors.extend(item.folder_colors);
            preferences.custom_icons.extend(item.custom_icons);
        }
        self.finish_item_customization_change(&[root]);
    }

    /// Operations report items one by one, so saves coalesce on idle.
    fn finish_item_customization_change(self: &Rc<Self>, roots: &[PathBuf]) {
        super::super::thumbnail::refresh_customized_icons_within(roots);
        if self.item_customizations.save_scheduled.replace(true) {
            return;
        }
        let manager = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(manager) = manager.upgrade() {
                manager.item_customizations.save_scheduled.set(false);
                manager.save_preferences();
            }
        });
    }
}

fn item_maps(preferences: &mut Preferences) -> [&mut HashMap<String, String>; 2] {
    [
        &mut preferences.folder_colors,
        &mut preferences.custom_icons,
    ]
}

/// Spells `path` the way stored keys are spelled, so component-wise prefix
/// checks never match `/a/doc` against `/a/docs2`.
fn key_path(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().as_ref())
}

fn take_within(map: &mut HashMap<String, String>, root: &Path) -> Vec<(String, String)> {
    if map.is_empty() {
        return Vec::new();
    }
    let keys = map
        .keys()
        .filter(|key| Path::new(key).starts_with(root))
        .cloned()
        .collect::<Vec<_>>();
    keys.into_iter()
        .filter_map(|key| map.remove_entry(&key))
        .collect()
}

/// Moves nothing, and drops nothing, when no key is within `from`, so a
/// repeated relocation is a no-op.
fn relocate(map: &mut HashMap<String, String>, from: &Path, to: &Path, merged: bool) -> bool {
    if map.is_empty() {
        return false;
    }
    let mut moving = Vec::new();
    let mut stale = Vec::new();
    for key in map.keys() {
        let path = Path::new(key);
        if path.starts_with(from) {
            moving.push(key.clone());
        } else if !merged && path.starts_with(to) {
            stale.push(key.clone());
        }
    }
    if moving.is_empty() {
        return false;
    }
    for key in stale {
        map.remove(&key);
    }
    for key in moving {
        let Some(value) = map.remove(&key) else {
            continue;
        };
        if merged && Path::new(&key) == from {
            continue;
        }
        if let Some(key) = rebased_key(&key, from, to) {
            map.insert(key, value);
        }
    }
    true
}

fn rebased_key(key: &str, from: &Path, to: &Path) -> Option<String> {
    let suffix = Path::new(key).strip_prefix(from).ok()?;
    let rebased = if suffix.as_os_str().is_empty() {
        to.to_path_buf()
    } else {
        to.join(suffix)
    };
    Some(rebased.to_string_lossy().into_owned())
}
