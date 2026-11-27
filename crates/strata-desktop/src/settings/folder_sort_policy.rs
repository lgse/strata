// SPDX-License-Identifier: MIT

use crate::model::{FolderSort, Location, SortOptions};

use super::{
    Preferences,
    folder_view_policy::remembered_key,
    folder_views::{FolderKey, FolderViews},
    sort_preferences, store_sort,
};

#[cfg(test)]
mod tests;

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
    Stored(Option<SortPrune>),
}

/// Sets the default sort (`set_default_sort_options`).
pub fn set_default_sort_options(settings: &mut Preferences, options: SortOptions) -> DefaultSort {
    if !store_sort(settings, options) {
        return DefaultSort::Unstorable;
    }
    DefaultSort::Stored(settings.remember_folder_views.then_some(SortPrune(options)))
}
