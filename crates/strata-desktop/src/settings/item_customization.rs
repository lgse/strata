// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use strata_core::model::{FolderColorValue, rebase_path};

use crate::services::ItemIdentity;

use super::{Preferences, trashed::TrashedValues};

/// Saved colours and icons kept in memory while a trashed item can still come back.
pub struct TrashedCustomizations {
    folder_colors: Vec<(String, String)>,
    custom_icons: Vec<(String, String)>,
}

pub const ICON_CHOICES: [(&str, &str); 16] = [
    ("strata-file-text", "Documents"),
    ("strata-download", "Downloads"),
    ("strata-file-code", "Code"),
    ("strata-file-archive", "Archive"),
    ("strata-image", "Pictures"),
    ("strata-video", "Videos"),
    ("strata-terminal", "Terminal"),
    ("strata-house", "Home"),
    ("strata-hard-drive", "Storage"),
    ("strata-network", "Network"),
    ("strata-monitor", "Computer"),
    ("strata-key", "Private"),
    ("strata-pin", "Pinned"),
    ("strata-play", "Media"),
    ("strata-settings", "Settings"),
    ("strata-list-checks", "Tasks"),
];

/// How a saved emoji choice starts: `emoji:🎵`.
pub const EMOJI_PREFIX: &str = "emoji:";

pub fn custom_emoji(name: &str) -> Option<&str> {
    let emoji = name.strip_prefix(EMOJI_PREFIX)?;
    (!emoji.is_empty() && emoji.len() <= 64 && !emoji.chars().any(char::is_control))
        .then_some(emoji)
}

/// Whether `name` is an icon an item may wear: a bundled choice or a safe emoji.
pub fn is_icon_choice(name: &str) -> bool {
    ICON_CHOICES.iter().any(|(icon, _)| *icon == name) || custom_emoji(name).is_some()
}

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Preferences {
    /// The colour saved for `path`, if it reads as one.
    pub fn item_color(&self, path: &Path) -> Option<FolderColorValue> {
        if self.folder_colors.is_empty() {
            return None;
        }
        FolderColorValue::parse(self.folder_colors.get(&key(path))?)
    }

    /// Saves `color` for every one of `paths`, or forgets theirs with `None`.
    pub fn set_item_colors(&mut self, paths: &[PathBuf], color: Option<&FolderColorValue>) {
        for path in paths {
            match color {
                Some(color) => {
                    self.folder_colors
                        .insert(key(path), color.to_preference_string());
                }
                None => {
                    self.folder_colors.remove(&key(path));
                }
            }
        }
    }

    /// The icon saved for `path`, if it is still one an item may wear.
    pub fn item_icon(&self, path: &Path) -> Option<&str> {
        if self.custom_icons.is_empty() {
            return None;
        }
        self.custom_icons
            .get(&key(path))
            .map(String::as_str)
            .filter(|name| is_icon_choice(name))
    }

    /// Saves `icon` for `path`, or forgets its icon with `None` or a name no item may wear.
    pub fn set_item_icon(&mut self, path: &Path, icon: Option<&str>) {
        match icon.filter(|name| is_icon_choice(name)) {
            Some(name) => {
                self.custom_icons.insert(key(path), name.to_owned());
            }
            None => {
                self.custom_icons.remove(&key(path));
            }
        }
    }

    /// Forgets both the colour and the icon of `path`.
    pub fn clear_item_customization(&mut self, path: &Path) {
        let key = key(path);
        self.folder_colors.remove(&key);
        self.custom_icons.remove(&key);
    }

    /// Whether `path` has a colour or an icon saved.
    pub fn is_item_customized(&self, path: &Path) -> bool {
        self.item_color(path).is_some() || self.item_icon(path).is_some()
    }

    pub fn relocate_item_customizations(
        &mut self,
        from: &Path,
        to: Option<&Path>,
        merged_into_existing: bool,
    ) -> bool {
        let from = key_path(from);
        let Some(to) = to.map(key_path) else {
            let colors = take_within(&mut self.folder_colors, &from);
            let icons = take_within(&mut self.custom_icons, &from);
            return !colors.is_empty() || !icons.is_empty();
        };
        if from == to {
            return false;
        }
        let mut changed = false;
        for map in item_maps(self) {
            changed |= relocate(map, &from, &to, merged_into_existing);
        }
        changed
    }

    pub fn forget_item_customizations(
        &mut self,
        root: &Path,
        trash_identity: Option<ItemIdentity>,
        trashed: &mut TrashedValues<TrashedCustomizations>,
    ) -> bool {
        let root = key_path(root);
        if let Some(identity) = trash_identity {
            trashed.forget_identity(identity);
        }
        let folder_colors = take_within(&mut self.folder_colors, &root);
        let custom_icons = take_within(&mut self.custom_icons, &root);
        let keys = folder_colors.len() + custom_icons.len();
        if let Some(identity) = trash_identity.filter(|_| keys > 0) {
            trashed.keep(
                root,
                identity,
                TrashedCustomizations {
                    folder_colors,
                    custom_icons,
                },
                keys,
            );
        }
        keys > 0
    }

    pub fn restore_item_customizations(
        &mut self,
        root: &Path,
        identity: ItemIdentity,
        trashed: &mut TrashedValues<TrashedCustomizations>,
    ) -> bool {
        let root = key_path(root);
        let Some(item) = trashed.take_restored(&root, identity) else {
            return false;
        };
        self.folder_colors.extend(item.folder_colors);
        self.custom_icons.extend(item.custom_icons);
        true
    }
}

fn item_maps(preferences: &mut Preferences) -> [&mut HashMap<String, String>; 2] {
    [
        &mut preferences.folder_colors,
        &mut preferences.custom_icons,
    ]
}

/// Stored keys are lossy UTF-8; compare paths in the same form.
pub fn key_path(path: &Path) -> PathBuf {
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

fn collect_relocated(
    map: &HashMap<String, String>,
    from: &Path,
    to: &Path,
    merged: bool,
) -> (Vec<String>, Vec<String>) {
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
    (moving, stale)
}

fn relocate(map: &mut HashMap<String, String>, from: &Path, to: &Path, merged: bool) -> bool {
    if map.is_empty() {
        return false;
    }
    let (moving, stale) = collect_relocated(map, from, to, merged);
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
        let keep = !merged || Path::new(&key) != from;
        if let Some(rebased) = keep.then(|| rebased_key(&key, from, to)).flatten() {
            map.insert(rebased, value);
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

#[cfg(test)]
mod tests;
