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
    /// Whether a press on an entry can start a file drag. Off for a read-only collection.
    pub drags: bool,
    /// How Ctrl+Shift+click combines its range with the selection. [`Extend::Replace`] by
    /// default, as Strata's pointer code does (Shift wins over Ctrl).
    pub ctrl_shift: Extend,
    /// An entry whose activation would change nothing: the folder whose child column is open in
    /// Columns (C2). A slow click on its name renames it instead of opening it again.
    pub already_open: Option<EntryId>,
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
            ctrl_shift: Extend::Replace,
            already_open: None,
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

    /// The same settings with `entry` as the one whose activation changes nothing.
    pub const fn already_open(mut self, entry: Option<EntryId>) -> Self {
        self.already_open = entry;
        self
    }
}
