// SPDX-License-Identifier: MIT

//! The List table over the pane's listing and collection.

use rune::{El, col, text};
use strata_components::{
    collection::element::{Collection, LABEL, Roles, RowCx, Slot},
    list_table::{ListParts, ListTable},
};
use strata_kit::look::Look;

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
            sort: crate::sort::headings::shown(app),
            look: &app.look,
        },
        |app: &mut App| &mut app.list.table,
    )
    .labels(app.list.menu_labels.clone())
    .cells_version(columns::cells_version)
    .on_layout(|app: &mut App, change, _| columns::keep_layout(app, change))
    .on_widths(|app: &mut App, widths, _| columns::keep_widths(app, widths))
    .on_sort(|app: &mut App, intent, _| crate::sort::headings::pressed(app, intent))
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

/// A filter's results in the table's place (A10's [`crate::filter::pane`]): a list of rows,
/// each its icon, its name, and its path under the folder while the filter searches the
/// subfolders, as GTK's `ResultKind::Rows`.
pub(super) fn results(app: &App) -> Collection<App> {
    let paths = app.filter.pane.recursive;
    single::results::<List>(app)
        .roles(Roles::LIST)
        .row_height(move |_: &App, look: &Look| match paths {
            true => (look.row.list + look.kit.ty.small.line_height + 2.0).round(),
            false => look.row.list,
        })
        .label(Slot::pure(results_label))
}

/// A result's name, and its path under it while there is one.
fn results_label(app: &App, rcx: &RowCx<'_>) -> El {
    let kit = &rcx.look.kit;
    let line = |words: String, key: &'static str, font, color| {
        text(words)
            .key(key)
            .a11y_hidden()
            .font(font)
            .color(color)
            .truncate_middle()
            .shrink()
            .min_w(0.0)
    };
    let name = line(
        rcx.entry.name.to_owned(),
        LABEL,
        kit.ty.ui_compact,
        kit.c.text_base,
    );
    let mut label = col().shrink().min_w(0.0).child(name);
    if let Some(path) = crate::filter::pane::path_of(app, rcx.entry.id) {
        // Keyed as a name label in a box of its own, so the QA harness reads it as GTK's
        // second label in the row.
        let path = line(path, LABEL, kit.ty.small, kit.c.text_muted);
        label = label.child(col().key("path").min_w(0.0).child(path));
    }
    label
}

/// The view: the pane's group around the table, or around a filter's results, with the
/// Columns button among the header's actions (`append_columns_menu` in
/// `src/ui/browser_modes/list_pane.rs`).
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    app.list.thumbs.set_scale(frame.cx.env().scale);
    let columns = columns_button(app, frame.layer);
    let body = match app.filter.pane.shows() {
        true => results(app).build(app),
        false => table(app).build(app, base(app), frame.layer),
    };
    Some(single::group::<List>(app, body, Some(columns)))
}

/// C4's Columns button: its key, which C4 makes of the table's.
const COLUMNS_BUTTON: &str = "list-columns-button";

/// The Columns button and its menu, mounted into `layer`. C4 draws the button, so Return
/// reaches it through the box around it.
fn columns_button(app: &App, layer: &mut Vec<El>) -> El {
    buttons::on_return_within(COLUMNS_BUTTON, table(app).columns_button(app, layer))
}
