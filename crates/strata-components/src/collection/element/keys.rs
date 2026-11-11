//! The keys the element gives its parts, so hosts, scripts and motion effects (C23) can name a
//! row, its icon and its label, and scroll requests can name the viewport.
//!
//! Keys are one namespace in Rune, and sibling keys must be unique. Rows are keyed by entry id
//! (`row-{id}`), which is unique inside one collection; two collections showing the same
//! folder (two panes) tell their rows apart by path: [`row_path`] starts at the collection's
//! own key. A folder row's drop zone reaches the host's binder keyed [`row_key`] with the
//! collection's key as its scope (W1's `ElementKey::within`), so a binder that finds zones by
//! key tells them apart the same way.

use crate::contract::{EntryId, EntrySource};

/// The scroll viewport's key, under the collection's key.
pub const VIEWPORT: &str = "viewport";

/// The virtual rows stack's key, under the viewport.
pub const ROWS: &str = "rows";

/// The row icon's key, under its row. A custom icon slot keys its part this too, so a motion
/// effect (C23) finds it by [`part_path`].
pub const ICON: &str = "icon";

/// The row label's key, under its row. A custom label slot keys its part this too.
pub const LABEL: &str = "label";

/// The live status line's key, under the collection's key.
pub const STATUS: &str = "status";

/// The stuck group header's key, under the collection's key: the header of the group at the
/// top of the viewport, drawn over it (`CollectionState::set_groups`). Its label is keyed
/// [`LABEL`].
pub const STUCK: &str = "stuck";

/// The key of the row (or grid cell) that shows entry `id`.
///
/// Used by: C1e, C5 (the row to keep on screen), C23 (flight sources), scripts.
pub fn row_key(id: EntryId) -> String {
    format!("row-{}", id.get())
}

/// The key of the header line of the group keyed `key` (`Group::key`).
///
/// Used by: C4, scripts.
pub fn header_key(key: u64) -> String {
    format!("head-{key}")
}

/// The key of the virtual line whose first entry is `first`: the row itself when each line
/// holds one entry, else `line-{id}`.
pub(super) fn line_key(first: EntryId, per_line: usize) -> String {
    if per_line <= 1 {
        row_key(first)
    } else {
        format!("line-{}", first.get())
    }
}

/// The full key path of the viewport in the collection keyed `scope`, for scroll requests.
pub fn viewport_path(scope: &str) -> String {
    format!("{scope}/{VIEWPORT}")
}

/// The full key path of entry `id`'s row in the collection keyed `scope`, when each line holds
/// one entry. It names the row even when another collection keys a row alike.
pub fn row_path(scope: &str, id: EntryId) -> String {
    format!("{scope}/{VIEWPORT}/{ROWS}/{}", row_key(id))
}

/// The full key path of a part (`ICON`, `LABEL`) of entry `id`'s row in the collection keyed
/// `scope`: what a motion effect flies from (C23).
pub fn part_path(scope: &str, id: EntryId, part: &str) -> String {
    format!("{}/{part}", row_path(scope, id))
}

/// The full key path of the entry at `index` in the collection keyed `scope`, whose lines
/// hold `per_line` entries each ([`CollectionState::per_line`]): `row_path` for one per line,
/// else the cell under its line (`{scope}/viewport/rows/line-{first}/row-{id}`). A grid's
/// paths change when it reflows to another count.
///
/// Used by: C1e (the active descendant), C3 (tile paths for X2's drop zones), scripts.
///
/// [`CollectionState::per_line`]: super::CollectionState::per_line
pub fn entry_path(scope: &str, source: &dyn EntrySource, index: usize, per_line: usize) -> String {
    let id = source.id_at(index);
    if per_line <= 1 {
        return row_path(scope, id);
    }
    let first = source.id_at(index / per_line * per_line);
    format!(
        "{scope}/{VIEWPORT}/{ROWS}/{}/{}",
        line_key(first, per_line),
        row_key(id)
    )
}

/// The full key path of the virtual line holding the entry at `index`: the row itself for one
/// entry per line. Rune holds and reveals virtual rows by these keys.
pub(super) fn line_path(
    scope: &str,
    source: &dyn EntrySource,
    index: usize,
    per_line: usize,
) -> String {
    let per = per_line.max(1);
    let first = source.id_at(index / per * per);
    format!("{scope}/{VIEWPORT}/{ROWS}/{}", line_key(first, per))
}
