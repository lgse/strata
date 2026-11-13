// SPDX-License-Identifier: MIT

//! The List table over the pane's listing and collection.

use rune::El;
use strata_components::{
    collection::element::Collection,
    list_table::{ListParts, ListTable},
};

use super::{List, VIEW, columns};
use crate::{App, single, slots::Frame};

pub(super) fn table(app: &App) -> ListTable<App> {
    ListTable::new(
        VIEW,
        columns::columns(&app.list.headings),
        |app: &App| ListParts {
            source: app.list.single.pane.source(),
            collection: app.list.single.pane.collection(),
            table: &app.list.table,
            layout: &app.list.layout,
            widths: &app.list.widths,
            // The browser's sort shows on its heading with A10's sorting.
            sort: None,
            look: &app.look,
        },
        |app: &mut App| &mut app.list.table,
    )
    .cells_version(columns::cells_version)
    .on_layout(|app: &mut App, change, _| columns::keep_layout(app, change))
    .on_widths(|app: &mut App, widths, _| columns::keep_widths(app, widths))
}

/// The pane's collection, with the rows the viewport shows recorded for the fills.
fn base(app: &App) -> Collection<App> {
    single::collection::<List>(app)
        .icon(app.list.thumbs.icon(&app.look))
        .on_visible(|app: &mut App, range| {
            // Recorded only: the part asks for the fills and syncs the thumbnails once a poll.
            app.list.fills.see(range.clone());
            app.list.thumbs.see(range);
            false
        })
}

/// The collection with the table's parts: what the poll ticks.
pub(super) fn ticked(app: &App) -> Collection<App> {
    table(app).collection(app, base(app))
}

/// The view: the pane's group around the table.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    app.list.thumbs.set_scale(frame.cx.env().scale);
    let table = table(app).build(app, base(app), frame.layer);
    Some(single::group::<List>(app, table))
}
