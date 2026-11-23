// SPDX-License-Identifier: MIT

use crate::contract::{EntryId, EntrySource};

/// The scroll viewport's key, under the collection's key.
pub const VIEWPORT: &str = "viewport";

/// The virtual rows stack's key, under the viewport.
pub const ROWS: &str = "rows";

/// The row icon's key, under its row.
pub const ICON: &str = "icon";

/// The row label's key, under its row. A custom label slot keys its part this too.
pub const LABEL: &str = "label";

/// The live status line's key, under the collection's key.
pub const STATUS: &str = "status";

/// The stuck group header's key, under the collection's key.
pub const STUCK: &str = "stuck";

/// Formats `prefix` followed by decimal `value` without `core::fmt` overhead on 100k-row key scans.
fn prefixed_u64(prefix: &str, mut value: u64) -> String {
    let mut digits = [0_u8; 20];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let tail = std::str::from_utf8(&digits[start..]).unwrap_or("0");
    let mut out = String::with_capacity(prefix.len() + tail.len());
    out.push_str(prefix);
    out.push_str(tail);
    out
}

/// The key of the row (or grid cell) that shows entry `id`.
pub fn row_key(id: EntryId) -> String {
    prefixed_u64("row-", id.get())
}

/// The key of the header line of the group keyed `key` (`Group::key`).
pub fn header_key(key: u64) -> String {
    prefixed_u64("head-", key)
}

pub(super) fn line_key(first: EntryId, per_line: usize) -> String {
    if per_line <= 1 {
        row_key(first)
    } else {
        prefixed_u64("line-", first.get())
    }
}

/// The full key path of the viewport in the collection keyed `scope`, for scroll requests.
pub fn viewport_path(scope: &str) -> String {
    format!("{scope}/{VIEWPORT}")
}

/// The full key path of entry `id`'s row in the collection keyed `scope`, when each line holds one entry.
pub fn row_path(scope: &str, id: EntryId) -> String {
    format!("{scope}/{VIEWPORT}/{ROWS}/{}", row_key(id))
}

/// The full key path of a part (`ICON`, `LABEL`) of entry `id`'s row in the collection keyed `scope`.
pub fn part_path(scope: &str, id: EntryId, part: &str) -> String {
    format!("{}/{part}", row_path(scope, id))
}

/// The full key path of the entry at `index` in the collection keyed `scope`.
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
