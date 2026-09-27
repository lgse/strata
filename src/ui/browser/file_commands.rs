// SPDX-License-Identifier: MIT

//! 10xer file commands. Each acts on the focused pane's fill, or on its cursor
//! item when nothing is filled, never on a hovered row or an open-path marker.

use std::rc::Rc;

use super::{
    BrowserView,
    clipboard::{self, copy_locations, copy_names},
    paths::is_trash_location,
};
use crate::model::FileEntry;

pub(crate) use super::transfer::ConflictFocus;

/// What **y** / **x** did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Yank {
    Nothing,
    Done,
    /// A cut was refused because an item cannot be removed from its place.
    Refused,
}

/// Why **a** did not create an item.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CreateRefusal {
    /// The focused folder does not take new items.
    Unsupported,
    Invalid(&'static str),
    Exists(String),
}

impl BrowserView {
    pub fn command_targets(&self) -> Vec<FileEntry> {
        if let Some(results) = self.selected_search_results() {
            if !results.is_empty() {
                return results;
            }
            return self.selected_search_result().into_iter().collect();
        }
        self.state.sync_mode_selection();
        self.focused_listing_depth()
            .map(|depth| self.state.browser.command_entries(depth))
            .unwrap_or_default()
    }

    pub fn yank_targets(&self, cut: bool) -> Yank {
        let entries = self.command_targets();
        if entries.is_empty() {
            return Yank::Nothing;
        }
        if !cut {
            self.state.copy_entries(&entries);
            return Yank::Done;
        }
        if self.state.cut_entries(&entries) {
            Yank::Done
        } else {
            Yank::Refused
        }
    }

    pub fn unyank(&self) {
        clipboard::unyank();
    }

    /// **c c** copies paths, **c n** names.
    pub fn copy_target_text(&self, names: bool) -> bool {
        let entries = self.command_targets();
        if entries.is_empty() {
            return false;
        }
        if names {
            copy_names(&entries);
        } else {
            copy_locations(&entries);
        }
        true
    }

    /// Pastes into the same destination as **Ctrl+V**. `nothing` runs when
    /// the clipboard holds no files or image.
    pub fn paste_preferring(&self, focus: ConflictFocus, nothing: Rc<dyn Fn()>) {
        if !clipboard::clipboard_may_paste() {
            nothing();
            return;
        }
        if let Some(location) = self.paste_location() {
            self.state.paste_preferring(location, focus, nothing);
        }
    }

    pub fn confirm_targets_delete(&self, permanent: bool) -> bool {
        let entries = self.command_targets();
        if entries.is_empty() {
            return false;
        }
        let permanent = permanent || self.focused_location_is_trash();
        self.state.request_confirmed_delete(entries, permanent);
        true
    }

    pub fn can_create_entry(&self) -> bool {
        self.new_entry_parent()
            .is_some_and(|(_, parent)| !is_trash_location(&parent) && !parent.is_recent_location())
    }

    /// Creates `text` in the keyboard-focused folder. A trailing `/` makes a
    /// folder and is stripped before validation; nothing else is trimmed.
    pub fn create_typed_entry(&self, text: &str) -> Result<(), CreateRefusal> {
        let (name, directory) = match text.strip_suffix('/') {
            Some(name) => (name, true),
            None => (text, false),
        };
        crate::services::validate_basename(name).map_err(CreateRefusal::Invalid)?;
        let Some((_, parent)) = self
            .new_entry_parent()
            .filter(|(_, parent)| !is_trash_location(parent) && !parent.is_recent_location())
        else {
            return Err(CreateRefusal::Unsupported);
        };
        // Creation itself is atomic; this only keeps the prompt open to fix the name.
        if let Some(path) = parent.native_path()
            && std::fs::symlink_metadata(path.join(name)).is_ok()
        {
            return Err(CreateRefusal::Exists(name.to_owned()));
        }
        self.state
            .browser
            .create_exact_entry(parent, name.to_owned(), directory);
        Ok(())
    }
}
