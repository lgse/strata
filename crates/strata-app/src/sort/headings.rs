// SPDX-License-Identifier: MIT

//! List's headings sort, GTK's `connect_sort` in `src/ui/browser_modes/list_headings.rs`: Name,
//! Size, Type and Modified are sortable, Mode is not. The sorted heading shows the browser's
//! sort, and a click asks C4's `next_direction` of it: the other way on the sorted heading,
//! ascending on another.

use strata_components::list_table::{ColumnId, SortDir, SortIntent, SortShown};
use strata_core::model::{SortDirection, SortKey};

use crate::App;

/// The headings that sort, by their column ids (`ListColumnId`'s serde names), and their keys.
const SORTED_BY: [(&str, SortKey); 4] = [
    ("name", SortKey::Name),
    ("size", SortKey::Size),
    ("type", SortKey::Type),
    ("modified", SortKey::Modified),
];

/// Whether the List heading `id` sorts.
pub(crate) fn sortable(id: &str) -> bool {
    SORTED_BY.iter().any(|(column, _)| *column == id)
}

/// The browser's sort of the List pane's column, as its headings show it: none when its field
/// has no heading.
pub(crate) fn shown(app: &App) -> Option<SortShown> {
    let depth = app.browser.active_depth()?;
    let prefs = app.browser.column_preferences(depth)?;
    let (column, _) = SORTED_BY.iter().find(|(_, key)| *key == prefs.sort_key)?;
    let dir = match prefs.sort_direction {
        SortDirection::Ascending => SortDir::Ascending,
        SortDirection::Descending => SortDir::Descending,
    };
    Some(SortShown {
        column: ColumnId::new(column),
        dir,
    })
}

/// A heading's click: the browser sorts the column by its field, in the direction asked.
pub(crate) fn pressed(app: &mut App, intent: SortIntent) {
    let Some(depth) = app.browser.active_depth() else {
        return;
    };
    let Some((_, key)) = SORTED_BY
        .iter()
        .find(|(column, _)| *column == intent.column.as_str())
    else {
        return;
    };
    let direction = match intent.dir {
        SortDir::Ascending => SortDirection::Ascending,
        SortDir::Descending => SortDirection::Descending,
    };
    app.browser.clone().set_sort(depth, *key, direction);
}
