//! What the editor selects when it opens.

use crate::contract::EntryKind;

/// What the editor selects when it opens.
///
/// Used by: hosts (C2, C3, C4, Strata).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Initial {
    /// A file's stem, the name before its last `.`; a folder's whole name. A rename.
    #[default]
    Stem,
    /// The whole name: a new item's default name, which typing replaces
    /// (`operations/create`).
    All,
}

/// The byte range the editor selects first in `name`, as Strata's `rename_stem_end` does:
/// a file selects up to its last `.` unless that `.` is the first character (`.bashrc`), so
/// `archive.tar.gz` selects `archive.tar`; a folder-like entry, a name without a `.` and
/// [`Initial::All`] select everything.
pub fn initial_selection(name: &str, kind: EntryKind, initial: Initial) -> (usize, usize) {
    if initial == Initial::All || kind.is_folder_like() {
        return (0, name.len());
    }
    let end = name.rfind('.').filter(|&dot| dot > 0).unwrap_or(name.len());
    (0, end)
}
