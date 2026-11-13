// SPDX-License-Identifier: MIT

//! The List table over the pane's listing and collection.

use rune::{El, KeyEvent, Tx};
use strata_components::{
    collection::element::Collection,
    list_table::{ListParts, ListTable},
};

use super::{List, VIEW, columns};
use crate::{App, buttons, single, slots::Frame};

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
    .labels(app.list.menu_labels.clone())
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

/// The view: the pane's group around the table, with the Columns button among the header's
/// actions (`append_columns_menu` in `src/ui/browser_modes/list_pane.rs`).
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    app.list.thumbs.set_scale(frame.cx.env().scale);
    let columns = columns_button(app, frame.layer);
    let table = table(app).build(app, base(app), frame.layer);
    Some(single::group::<List>(app, table, Some(columns)))
}

/// C4's Columns button: its key, which C4 makes of the table's.
const COLUMNS_BUTTON: &str = "list-columns-button";

/// The Columns button and its menu, mounted into `layer`. C4 draws the button, so Return
/// reaches it through the box around it, turned into the Space that presses it, as
/// `buttons::on_return` does for the window's own.
fn columns_button(app: &App, layer: &mut Vec<El>) -> El {
    table(app)
        .columns_button(app, layer)
        .on_key(|_: &mut App, event: &KeyEvent, tx: &mut Tx| {
            tx.focused() == Some(COLUMNS_BUTTON) && buttons::pressed(event, tx)
        })
}
