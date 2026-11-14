// SPDX-License-Identifier: MIT

//! What the per-folder views keep beside the defaults in `settings.toml`: which folder is
//! remembered, where a changed value is kept, and what a new default prunes. Both interfaces
//! use it, by the owner's ruling of 2026-11-12 (`docs/rune/parity/owner.md`, B2): the GTK
//! manager's methods in `src/ui/preferences/folder_view_settings.rs` wrap it with their store,
//! bindings and saves, and `strata-app`'s `folder_views` with its own.
//!
//! The functions read and change [`Preferences`] and say what follows for the folders' views,
//! without holding the views: a caller reads its store only for a folder this says is
//! remembered, as the GTK manager loads its store only then. Saving `settings.toml` and
//! scheduling the views' write stay the caller's.
//!
//! - **The gate.** A folder's view is read and written only while `remember_folder_views` is
//!   on, and only for a location [`key_for_location`] keys ([`remembered_key`]).
//! - **What is kept.** A size equal to the default is not stored, so the folder follows later
//!   defaults; with folders' views off a changed size is the default ([`keep_icons_size`]).
//! - **What a new default prunes.** A default changed while folders' views are remembered
//!   drops the values that now equal it, and turning them back on drops those that came to
//!   equal it meanwhile ([`Prune`]).
//!
//! The sort's policy is beside it, in [`super::folder_sort_policy`]; [`Prune`] prunes a sort
//! when folders' views are remembered again.

use crate::model::{Location, SortOptions};

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, Preferences,
    folder_views::{FolderKey, FolderViews, key_for_location},
    sort_preferences,
};

#[cfg(test)]
mod tests;

/// The key `location`'s view is kept under: none while folders' views are not remembered,
/// or for a location with no key.
pub fn remembered_key(settings: &Preferences, location: &Location) -> Option<FolderKey> {
    if !settings.remember_folder_views {
        return None;
    }
    key_for_location(location)
}

/// The size `location` shows at: its own, read through `own` only when it is remembered,
/// else the default.
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

/// Where `size`, changed while `location` shows, is kept (`set_folder_icons_size`). It is
/// held between 32 and 256.
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

/// Sets the default size, held between 32 and 256 (`set_default_icons_size`): what to prune
/// while folders' views are remembered.
pub fn set_default_icons_size(settings: &mut Preferences, size: i32) -> Option<Prune> {
    let size = size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
    settings.icons_thumbnail_size = size;
    settings
        .remember_folder_views
        .then_some(Prune::IconsSize(size))
}

/// Turns folders' views on or off (`set_remember_folder_views`). Defaults changed while they
/// were off can equal saved values, which are dropped when they are turned back on.
pub fn set_remember_folder_views(settings: &mut Preferences, enabled: bool) -> Option<Prune> {
    settings.remember_folder_views = enabled;
    enabled.then(|| Prune::Defaults {
        sort: sort_preferences(settings).sort_options(),
        icons_size: settings.icons_thumbnail_size,
    })
}
