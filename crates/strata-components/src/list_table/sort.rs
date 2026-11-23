// SPDX-License-Identifier: MIT

pub use rune_kit::SortDir;

use super::column::ColumnId;

/// The host's sort, as the headings show it: the column its field is (whether or not that column shows) and the direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortShown {
    /// The sorted column.
    pub column: ColumnId,
    /// Which way.
    pub dir: SortDir,
}

/// A click on a sortable heading: sort by `column`, `dir`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortIntent {
    /// The column clicked.
    pub column: ColumnId,
    /// The direction it asks for.
    pub dir: SortDir,
}

/// What a click on `column`'s heading asks for.
pub fn next_direction(shown: Option<&SortShown>, column: &ColumnId) -> SortDir {
    match shown {
        Some(s) if s.column == *column => s.dir.flip(),
        _ => SortDir::Ascending,
    }
}
