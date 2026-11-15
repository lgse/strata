// SPDX-License-Identifier: MIT

//! An item's saved customization: its colour (`folder_colors`) and its icon (`custom_icons`),
//! each keyed by the item's native path. Both interfaces read and write them here; the GTK
//! manager's getters and setters (`src/ui/preferences/file_settings.rs`) call these and keep
//! what is GTK's around them, saving and redrawing the icons.

use std::path::{Path, PathBuf};

use strata_core::model::FolderColorValue;

use super::Preferences;

/// The bundled icons an item may wear, each by its catalog name, with its label in the
/// catalogs (`CUSTOMIZATION_CHOICES` in `src/assets.rs`, which a GTK test holds to this list).
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

/// The emoji a saved icon names, if it is one that is safe to draw: not empty, at most 64
/// bytes, and no control character.
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
}

#[cfg(test)]
mod tests;
