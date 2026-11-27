// SPDX-License-Identifier: MIT

use std::time::Duration;

use crate::collection::selection::Extend;
use crate::contract::{EntryId, EntryKind};

/// How many clicks open an entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClickCount {
    /// One click opens it, on release.
    One,
    /// A double click opens it; one click only selects.
    #[default]
    Two,
}

/// The click count for files and for folders in one view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClickModes {
    /// Regular files, special files and links that don't lead to a folder.
    pub files: ClickCount,
    /// Folders and links to them ([`EntryKind::is_folder_like`]).
    pub folders: ClickCount,
}

impl ClickModes {
    /// `files` for files, `folders` for folders.
    pub const fn new(files: ClickCount, folders: ClickCount) -> Self {
        Self { files, folders }
    }

    /// The count that opens an entry of `kind`. A broken link counts as a file.
    pub const fn for_kind(self, kind: EntryKind) -> ClickCount {
        if kind.is_folder_like() {
            self.folders
        } else {
            self.files
        }
    }
}

/// The pointer machine's settings: the double-click interval from the desktop, the click modes from the preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PointerSettings {
    /// Two presses closer than this pair into a double click, and a slow click waits this long before it renames.
    pub double_click: Duration,
    /// How far apart (points, per axis, in window coordinates) two presses may be and still pair.
    pub double_click_distance: f32,
    /// How far (points, per axis) a press travels before it is a drag or a marquee.
    pub drag_threshold: f32,
    /// How many clicks open files and folders.
    pub modes: ClickModes,
    /// Whether a slow second click on a name renames. Off in file choosers and in Trash.
    pub rename: bool,
    /// Whether a press on an entry can start a file drag.
    pub drags: bool,
    /// Whether blank row or card space on an already-selected entry starts a file drag when `drags` is also enabled.
    pub drag_selected_rows: bool,
    /// Whether more than one entry may be selected (Ctrl/Shift clicks and marquee bands).
    pub multiple: bool,
    /// How Ctrl+Shift+click combines its range with the selection.
    pub ctrl_shift: Extend,
    /// An entry whose activation would change nothing: the folder whose child column is open in Columns.
    pub already_open: Option<EntryId>,
    /// Whether a missing Shift anchor stays missing through a secondary press and a Shift+press.
    pub missing_anchor_stays: bool,
}

impl Default for PointerSettings {
    fn default() -> Self {
        Self {
            double_click: Duration::from_millis(400),
            double_click_distance: 5.0,
            drag_threshold: rune::drag_out::DRAG_THRESHOLD,
            modes: ClickModes::default(),
            rename: true,
            drags: true,
            drag_selected_rows: true,
            multiple: true,
            ctrl_shift: Extend::Replace,
            already_open: None,
            missing_anchor_stays: false,
        }
    }
}

impl PointerSettings {
    /// The same settings with `modes`.
    pub const fn modes(mut self, modes: ClickModes) -> Self {
        self.modes = modes;
        self
    }

    /// The same settings with the double-click interval `interval`.
    pub const fn double_click(mut self, interval: Duration) -> Self {
        self.double_click = interval;
        self
    }

    /// The same settings with slow-click rename on or off.
    pub const fn rename(mut self, rename: bool) -> Self {
        self.rename = rename;
        self
    }

    /// The same settings with outgoing file drags on or off.
    pub const fn drags(mut self, drags: bool) -> Self {
        self.drags = drags;
        self
    }

    /// The same settings with selected-row blank space reserved for file drags, or kept as marquee selection space.
    pub const fn drag_selected_rows(mut self, drag: bool) -> Self {
        self.drag_selected_rows = drag;
        self
    }

    /// The same settings with multiple selection on or off.
    pub const fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// The same settings with a missing anchor staying missing through a secondary press and a Shift+press, or not.
    pub const fn missing_anchor_stays(mut self, stays: bool) -> Self {
        self.missing_anchor_stays = stays;
        self
    }

    /// The same settings with `entry` as the one whose activation changes nothing.
    pub const fn already_open(mut self, entry: Option<EntryId>) -> Self {
        self.already_open = entry;
        self
    }
}
