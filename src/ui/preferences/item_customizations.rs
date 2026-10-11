// SPDX-License-Identifier: MIT

//! Folder colors and custom icons are keyed by absolute path, so the file
//! operations Strata performs carry, drop or bring back those keys.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::glib;

use crate::{model::rebase_path, services::TrashedOriginal};

use super::{PreferenceManager, Preferences, trashed::TrashedValues};

#[derive(Default)]
pub(super) struct ItemCustomizationState {
    trashed: RefCell<TrashedValues<TrashedCustomizations>>,
    save_scheduled: Cell<bool>,
}

struct TrashedCustomizations {
    folder_colors: Vec<(String, String)>,
    custom_icons: Vec<(String, String)>,
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
            let keys = folder_colors.len() + custom_icons.len();
            if let Some(identity) = trash_identity.filter(|_| keys > 0) {
                trashed.keep(
                    root.clone(),
                    identity,
                    TrashedCustomizations {
                        folder_colors,
                        custom_icons,
                    },
                    keys,
                );
            }
            keys > 0
        };
        if changed {
            self.finish_item_customization_change(&[root]);
        }
    }

    /// Re-applies what [`Self::forget_item_customizations`] kept for the very
    /// item now back at `root`; another item restored there gets nothing.
    pub fn restore_item_customizations(self: &Rc<Self>, root: &Path) {
        let root = key_path(root);
        let item = self
            .item_customizations
            .trashed
            .borrow_mut()
            .take_restored(&root);
        let Some(item) = item else {
            return;
        };
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

/// Stored keys are lossy UTF-8; compare paths in the same form.
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
    Some(
        rebase_path(Path::new(key), from, to)?
            .to_string_lossy()
            .into_owned(),
    )
}
