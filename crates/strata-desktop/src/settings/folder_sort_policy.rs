// SPDX-License-Identifier: MIT

//! The sort's share of the per-folder views' policy, beside the size's
//! ([`super::folder_view_policy`]): what a folder's sort resolves to, where a chosen sort is
//! kept, and what a new default sort prunes. Both interfaces use it, by the owner's ruling of
//! 2026-11-12 (`docs/rune/parity/owner.md`, B2): the GTK manager's `resolve_folder_sort`,
//! `set_folder_sort_options`, `reset_folder_sort` and `set_default_sort_options`
//! (`src/ui/preferences/folder_view_settings.rs`) wrap it with their store and saves, and
//! `strata-app`'s `folder_views` with its own.
//!
//! As for the size, the functions read and change [`Preferences`] and say what follows for the
//! folders' views. A caller reads its store only for a folder [`remembered_key`] gives a key,
//! and saving `settings.toml` and scheduling the views' write stay the caller's.
//!
//! - **What a folder resolves to** ([`folder_sort`]): its own saved sort, or the default; a
//!   folder only opened counts as used, for the least-recently-used limit.
//! - **What is kept** ([`keep_folder_sort`]): a sort that cannot be stored (device order,
//!   recency) is not kept, and one equal to the default is kept as none, so the folder
//!   follows later defaults. [`forget_folder_sort`] drops a folder's own.
//! - **What a new default prunes** ([`set_default_sort_options`]): while folders' views are
//!   remembered, the folders' sorts that now equal it.

use crate::model::{FolderSort, Location, SortOptions};

use super::{
    Preferences,
    folder_view_policy::remembered_key,
    folder_views::{FolderKey, FolderViews},
    sort_preferences, store_sort,
};

#[cfg(test)]
mod tests;

/// The sort of the remembered folder `key` (`resolve_folder_sort`): its own, or the default.
/// With `opened`, the folder counts as used now; the second value says whether that changed
/// the views, which then want writing, as after the folder was only opened.
pub fn folder_sort(views: &mut FolderViews, key: &FolderKey, opened: bool) -> (FolderSort, bool) {
    let touched = opened && views.touch(key);
    let sort = match views.view(key).sort {
        Some((sort_key, direction, comparison)) => {
            FolderSort::Saved(sort_key, direction, comparison)
        }
        None => FolderSort::Default,
    };
    (sort, touched)
}

/// A folder's sort to keep in the folders' views: its key, and its own sort, or none to follow
/// the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortKept {
    pub key: FolderKey,
    pub sort: Option<SortOptions>,
}

impl SortKept {
    /// Keeps it. Says whether the views changed.
    pub fn apply(self, views: &mut FolderViews) -> bool {
        views.update(&self.key, |view| view.sort = self.sort)
    }
}

/// Where `options`, chosen in `location`, are kept (`set_folder_sort_options`): nowhere for a
/// sort that cannot be stored or a folder that is not remembered, and as none when they equal
/// the default.
pub fn keep_folder_sort(
    settings: &Preferences,
    location: &Location,
    options: SortOptions,
) -> Option<SortKept> {
    options.0.stored_name()?;
    let key = remembered_key(settings, location)?;
    let default = sort_preferences(settings).sort_options();
    Some(SortKept {
        key,
        sort: Some(options).filter(|sort| *sort != default),
    })
}

/// Drops `location`'s own sort, so it follows the default (`reset_folder_sort`): nothing for a
/// folder that is not remembered.
pub fn forget_folder_sort(settings: &Preferences, location: &Location) -> Option<SortKept> {
    let key = remembered_key(settings, location)?;
    Some(SortKept { key, sort: None })
}

/// The folders' sorts a new default makes equal to it, to drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortPrune(SortOptions);

impl SortPrune {
    /// Drops them. Says whether anything went.
    pub fn apply(self, views: &mut FolderViews) -> bool {
        views.prune_default_sort(self.0)
    }
}

/// What setting a default sort did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultSort {
    /// The sort cannot be a default (device order, recency): nothing changed.
    Unstorable,
    /// The default is set, for the caller to save, with what the folders' views drop while
    /// they are remembered.
    Stored(Option<SortPrune>),
}

/// Sets the default sort (`set_default_sort_options`).
pub fn set_default_sort_options(settings: &mut Preferences, options: SortOptions) -> DefaultSort {
    if !store_sort(settings, options) {
        return DefaultSort::Unstorable;
    }
    DefaultSort::Stored(settings.remember_folder_views.then_some(SortPrune(options)))
}
