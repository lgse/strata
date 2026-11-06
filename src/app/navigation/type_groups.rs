// SPDX-License-Identifier: MIT

//! The order of a listing grouped by file type. The view's sorter and the
//! automatic cursor share it, so a load's cursor lands on the first row shown.

use std::cmp::Ordering;

use crate::{
    model::{SortDirection, SortKey, ViewPreferences},
    services::{EntryType, FOLDER_TYPE_NAME, OTHER_TYPE_NAME, entry_type},
};

use super::{ColumnState, NavigationState, compare_display_names};

/// Orders two type-group labels as a grouped listing shows them. An empty label
/// belongs to no group.
pub(crate) fn compare_type_groups(
    left: &str,
    right: &str,
    preferences: ViewPreferences,
) -> Ordering {
    if preferences.sort_key != SortKey::Type {
        return compare_ranked_groups(left, right);
    }
    if left.is_empty() || right.is_empty() {
        return left.cmp(right);
    }
    if preferences.folders_first {
        let folders = (right == FOLDER_TYPE_NAME).cmp(&(left == FOLDER_TYPE_NAME));
        if folders != Ordering::Equal {
            return folders;
        }
    }
    let order = (left == OTHER_TYPE_NAME)
        .cmp(&(right == OTHER_TYPE_NAME))
        .then_with(|| compare_display_names(left, right));
    match preferences.sort_direction {
        SortDirection::Ascending => order,
        SortDirection::Descending => order.reverse(),
    }
}

/// Folders first and Other last, whatever the listing is sorted by.
fn compare_ranked_groups(left: &str, right: &str) -> Ordering {
    fn rank(label: &str) -> u8 {
        match label {
            "" => 0,
            FOLDER_TYPE_NAME => 1,
            OTHER_TYPE_NAME => 3,
            _ => 2,
        }
    }
    rank(left).cmp(&rank(right)).then_with(|| {
        left.chars()
            .flat_map(char::to_lowercase)
            .cmp(right.chars().flat_map(char::to_lowercase))
    })
}

impl ColumnState {
    /// The view groups this listing by file type. Recent keeps its own order. GTK
    /// 4.22 cannot safely section interleaved camera batches, and device order must
    /// not reshuffle once complete, so a camera folder groups only a complete
    /// snapshot that is not in device order.
    pub(super) fn shows_type_groups(&self) -> bool {
        let unsettled_camera = self.location.is_camera_photo_root()
            && (self.load_state == super::LoadState::Loading
                || self.preferences.sort_key == SortKey::DeviceOrder);
        self.type_grouping && !self.location.is_recent_root() && !unsettled_camera
    }

    /// The first row the view shows: the first visible entry, or in a grouped
    /// listing the first entry of the first group. Groups keep listing order.
    pub(super) fn first_displayed_entry(&self) -> Option<usize> {
        // A type sort already lists the entries in group order.
        if !self.shows_type_groups() || self.preferences.sort_key == SortKey::Type {
            return self.first_visible();
        }
        // Under any other sort the Folder group comes first, so a listed folder
        // spares a type lookup for every entry.
        self.first_visible_folder()
            .or_else(|| self.first_in_file_groups())
    }

    fn first_visible_folder(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.is_directory() && self.shows(entry))
    }

    /// The last row the view shows: the last visible entry, or in a grouped listing
    /// the last entry of the last group.
    pub(super) fn last_displayed_entry(&self) -> Option<usize> {
        if !self.shows_type_groups() || self.preferences.sort_key == SortKey::Type {
            return self.last_visible();
        }
        // The Folder group comes first, so with no files the last folder is last.
        self.last_in_file_groups().or_else(|| self.last_visible())
    }

    fn first_in_file_groups(&self) -> Option<usize> {
        // `min_by` keeps the first of equal groups, the earliest in listing order.
        self.visible_file_groups()
            .min_by(|(left, _), (right, _)| self.compare_groups(left, right))
            .map(|(_, position)| position)
    }

    fn last_in_file_groups(&self) -> Option<usize> {
        // `max_by` keeps the last of equal groups, the latest in listing order.
        self.visible_file_groups()
            .max_by(|(left, _), (right, _)| self.compare_groups(left, right))
            .map(|(_, position)| position)
    }

    fn visible_file_groups(&self) -> impl Iterator<Item = (EntryType, usize)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.is_directory() && self.shows(entry))
            .map(|(position, entry)| (entry_type(entry), position))
    }

    fn compare_groups(&self, left: &EntryType, right: &EntryType) -> Ordering {
        compare_type_groups(left.description(), right.description(), self.preferences)
    }
}

impl NavigationState {
    /// Whether the presentation groups listings by file type. Each column applies
    /// it as `ColumnState::shows_type_groups` decides.
    pub fn set_type_grouping(&mut self, grouped: bool) {
        self.type_grouping = grouped;
        for column in &mut self.columns {
            column.type_grouping = grouped;
        }
    }

    pub fn shows_type_groups(&self, depth: usize) -> bool {
        self.columns
            .get(depth)
            .is_some_and(ColumnState::shows_type_groups)
    }
}
