//! The sort the headings show, and the sort a heading click asks for. The table never sorts:
//! the host does, on [`SortIntent`], and gives the table its sort back as [`SortShown`].

pub use rune_kit::SortDir;

use super::column::ColumnId;

/// The host's sort, as the headings show it: the column its field is (whether or not that
/// column shows) and the direction. `None` when the field has no column (Extension, Created,
/// Recency, Unsorted): then no heading shows an arrow.
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortShown {
    /// The sorted column.
    pub column: ColumnId,
    /// Which way.
    pub dir: SortDir,
}

/// A click on a sortable heading: sort by `column`, `dir`. The host reorders its listing,
/// changes its listing version and syncs the collection.
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortIntent {
    /// The column clicked.
    pub column: ColumnId,
    /// The direction it asks for.
    pub dir: SortDir,
}

/// What a click on `column`'s heading asks for: the other way when it is the sorted column,
/// else ascending, as Strata's `next_direction` (`src/ui/browser_modes/list_headings.rs`).
pub fn next_direction(shown: Option<&SortShown>, column: &ColumnId) -> SortDir {
    match shown {
        Some(s) if s.column == *column => s.dir.flip(),
        _ => SortDir::Ascending,
    }
}
