// SPDX-License-Identifier: MIT

//! What the pointer machine reads on every input: the click modes, the double-click interval
//! and the drag threshold. The host passes them with `Collection::settings`.

use std::time::Duration;

use crate::collection::selection::Extend;
use crate::contract::{EntryId, EntryKind};

/// How many clicks open an entry.
///
/// Used by: C2, C3, C4, and hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClickCount {
    /// One click opens it, on release.
    One,
    /// A double click opens it; one click only selects.
    #[default]
    Two,
}

/// The click count for files and for folders in one view. Strata sets it per view (Columns,
/// Icons, List) from its preferences; that policy, and its defaults, are the host's.
///
/// Used by: C2, C3, C4, and hosts.
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

/// The pointer machine's settings: the double-click interval from the desktop (P5), the click
/// modes from the preferences, and the per-view switches. The host passes the current value
/// each time it builds the view (`Collection::settings(PointerSettings { .. })`), and the
/// controller reads it on every input (`Ctx::settings`); with none, it uses the default.
///
/// Fields may be added: start from [`PointerSettings::default`] and set fields, or use the
/// builder methods.
///
/// Used by: C2, C3, C4, C14b, C18, and hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PointerSettings {
    /// Two presses closer than this pair into a double click, and a slow click waits this long
    /// before it renames. 400 ms by default: GTK's and GNOME's `double-click`, which Strata
    /// reads (`gtk_double_click_time`). Rune's own interval is fixed at 0.5 s on Linux (S5
    /// U13), so the machine counts clicks itself.
    pub double_click: Duration,
    /// How far apart (points, per axis, in window coordinates) two presses may be and still
    /// pair. 5 pt by default: GTK's `gtk-double-click-distance`, which Strata reads.
    pub double_click_distance: f32,
    /// How far (points, per axis) a press travels before it is a drag or a marquee:
    /// `rune::drag_out::DRAG_THRESHOLD`, 8 pt on Linux, by default. Strata compares each axis
    /// (`src/ui/pointer.rs` `exceeds_drag_threshold`), where Rune measures straight-line travel.
    pub drag_threshold: f32,
    /// How many clicks open files and folders.
    pub modes: ClickModes,
    /// Whether a slow second click on a name renames. Off in file choosers and in Trash.
    pub rename: bool,
    /// Whether a press on an entry can start a file drag. Off for a read-only collection and
    /// file choosers.
    pub drags: bool,
    /// Whether blank row or card space on an already-selected entry starts a file drag when
    /// [`drags`](Self::drags) is also enabled. True by default (Columns and Icons); false in
    /// List, where unused Name space, row padding, and metadata columns remain marquee origins
    /// before and after selection.
    pub drag_selected_rows: bool,
    /// Whether more than one entry may be selected (Ctrl/Shift clicks and marquee bands).
    /// True by default; false in single-selection file choosers.
    pub multiple: bool,
    /// How Ctrl+Shift+click combines its range with the selection. [`Extend::Replace`] by
    /// default, as Strata's pointer code does (Shift wins over Ctrl).
    pub ctrl_shift: Extend,
    /// An entry whose activation would change nothing: the folder whose child column is open in
    /// Columns (C2). A slow click on its name renames it instead of opening it again.
    pub already_open: Option<EntryId>,
    /// Whether a missing Shift anchor stays missing through a secondary press and a
    /// Shift+press, so each Shift+click selects its entry alone until a plain or Ctrl press
    /// sets an anchor. Off by default: either press anchors its entry, as Strata's browser
    /// views do. On for Strata's search and filter results, which keep an anchor of their own
    /// that neither press sets (`pointer_selection`; findings lgse/strata#2375 and #2431).
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

    /// The same settings with the double-click interval `interval` (from P5).
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

    /// The same settings with selected-row blank space reserved for file drags, or kept as
    /// marquee selection space.
    pub const fn drag_selected_rows(mut self, drag: bool) -> Self {
        self.drag_selected_rows = drag;
        self
    }

    /// The same settings with multiple selection on or off.
    pub const fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// The same settings with a missing anchor staying missing through a secondary press and
    /// a Shift+press, or not.
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
