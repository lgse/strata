// SPDX-License-Identifier: MIT

//! The List table over the pane's listing and collection.

use rune::El;
use strata_components::{
    collection::element::Collection,
    list_table::{Column as TableColumn, ColumnId, ColumnLayout, ListParts, ListTable},
};
use strata_core::i18n::tr;
use strata_desktop::settings::ListColumnId;

use super::{List, VIEW};
use crate::{App, single, slots::Frame};

/// The Name column's id, as the settings save it (`ListColumnId`'s serde name).
fn name_id() -> ColumnId {
    ColumnId::new("name")
}

/// The Name column's heading, translated once: the language changes only on restart.
pub(super) fn name_heading() -> String {
    tr(ListColumnId::Name.title())
}

/// The columns the table shows, in order: the Name column alone, until the listing carries
/// what the others show.
pub(super) fn layout() -> ColumnLayout {
    ColumnLayout::new([name_id()])
}

pub(super) fn table(app: &App) -> ListTable<App> {
    let columns = vec![
        TableColumn::primary(name_id().as_str(), app.list.name.clone())
            // The Name column's default width and minimum: `src/ui/browser_modes/list_columns.rs`.
            .width(160.0)
            .min(160.0),
    ];
    ListTable::new(
        VIEW,
        columns,
        |app: &App| ListParts {
            source: app.list.single.pane.source(),
            collection: app.list.single.pane.collection(),
            table: &app.list.table,
            layout: &app.list.layout,
            widths: &app.list.widths,
            sort: None,
            look: &app.look,
        },
        |app: &mut App| &mut app.list.table,
    )
    // A width set by hand holds in this window. Saving it (`browser_list_columns`) waits on
    // the other columns, whose widths the setting keeps with it.
    .on_widths(|app: &mut App, widths, _| app.list.widths = widths)
}

/// The collection with the table's parts: what the poll ticks.
pub(super) fn ticked(app: &App) -> Collection<App> {
    table(app).collection(app, single::collection::<List>(app))
}

/// The view: the pane's group around the table.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let table = table(app).build(app, single::collection::<List>(app), frame.layer);
    Some(single::group::<List>(app, table))
}
