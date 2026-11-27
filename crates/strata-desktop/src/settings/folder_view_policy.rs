// SPDX-License-Identifier: MIT

use crate::model::{Location, SortOptions};

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, Preferences,
    folder_views::{FolderKey, FolderViews, key_for_location},
    sort_preferences,
};

#[cfg(test)]
mod tests;

pub fn remembered_key(settings: &Preferences, location: &Location) -> Option<FolderKey> {
    if !settings.remember_folder_views {
        return None;
    }
    key_for_location(location)
}

pub fn icons_size_for(
    settings: &Preferences,
    location: Option<&Location>,
    own: impl FnOnce(&FolderKey) -> Option<i32>,
) -> i32 {
    location
        .and_then(|location| remembered_key(settings, location))
        .and_then(|key| own(&key))
        .unwrap_or(settings.icons_thumbnail_size)
}

/// Where a changed size is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconsSizeKept {
    /// Folders' views are not remembered: the size is the default, in `settings.toml`.
    Default(i32),
    /// The folder's own, or `None` where it equals the default.
    Folder(FolderKey, Option<i32>),
    /// A location with no key keeps a changed size only while it is shown.
    Shown,
}

pub fn keep_icons_size(
    settings: &Preferences,
    location: Option<&Location>,
    size: i32,
) -> IconsSizeKept {
    let size = size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
    if !settings.remember_folder_views {
        return IconsSizeKept::Default(size);
    }
    match location.and_then(key_for_location) {
        Some(key) => {
            IconsSizeKept::Folder(key, (size != settings.icons_thumbnail_size).then_some(size))
        }
        None => IconsSizeKept::Shown,
    }
}

/// What a changed default drops from the folders' views: the values that now equal it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prune {
    /// A new default size.
    IconsSize(i32),
    /// Folders' views remembered again: the sorts and sizes that equal the defaults.
    Defaults { sort: SortOptions, icons_size: i32 },
}

impl Prune {
    /// Drops what equals the default. Says whether anything went.
    pub fn apply(self, views: &mut FolderViews) -> bool {
        match self {
            Self::IconsSize(size) => views.prune_default_icons_size(size),
            Self::Defaults { sort, icons_size } => {
                views.prune_default_sort(sort) | views.prune_default_icons_size(icons_size)
            }
        }
    }

    /// Whether pruning can reach the folders' sorts, which reorder open folders.
    pub fn reaches_sorts(self) -> bool {
        matches!(self, Self::Defaults { .. })
    }
}

pub fn set_default_icons_size(settings: &mut Preferences, size: i32) -> Option<Prune> {
    let size = size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
    settings.icons_thumbnail_size = size;
    settings
        .remember_folder_views
        .then_some(Prune::IconsSize(size))
}

pub fn set_remember_folder_views(settings: &mut Preferences, enabled: bool) -> Option<Prune> {
    settings.remember_folder_views = enabled;
    enabled.then(|| Prune::Defaults {
        sort: sort_preferences(settings).sort_options(),
        icons_size: settings.icons_thumbnail_size,
    })
}
