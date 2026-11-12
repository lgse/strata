// SPDX-License-Identifier: MIT

//! The List table over the pane's listing and collection, with Strata's words.

use rune::{El, col, text};
use strata_components::{
    collection::{
        element::{Collection, CollectionState, Describe},
        pointer::PointerSettings,
    },
    contract::EntrySource,
    list_table::{Column as TableColumn, ColumnId, ColumnLayout, ListParts, ListTable},
};
use strata_core::i18n::tr;
use strata_desktop::settings::ListColumnId;

use super::{VIEW, intents};
use crate::{
    App,
    listing::{Column, Load},
    rename,
    slots::Frame,
};

/// The view's words, translated once: the language changes only on restart.
pub(super) struct Labels {
    /// `view_name` in `src/ui/accessibility.rs`: the QA harness finds the pane by it.
    pane: String,
    /// `ENTRY_CONTAINER_DESCRIPTION` there: what the table of entries is.
    list: String,
    /// The Name column's heading (`list_headings.rs` in `src/ui/browser_modes`).
    name: String,
}

impl Labels {
    pub(super) fn translated() -> Self {
        Self {
            pane: tr("List view"),
            list: tr("Files"),
            name: tr(ListColumnId::Name.title()),
        }
    }
}

/// The Name column's id, as the settings save it (`ListColumnId`'s serde name).
fn name_id() -> ColumnId {
    ColumnId::new("name")
}

/// The columns the table shows, in order: the Name column alone, until the listing carries
/// what the others show.
pub(super) fn layout() -> ColumnLayout {
    ColumnLayout::new([name_id()])
}

/// The folder the table shows, as the pane and the table are named: `display_name`, as GTK's
/// `describe_pane` and `describe_entry_container` name them.
fn title(app: &App) -> String {
    app.list
        .pane
        .listing()
        .map(Column::title)
        .unwrap_or_default()
        .to_owned()
}

/// What the table shows in place of rows when its folder could not be read: the browser's
/// message.
fn failure(app: &App) -> Option<El> {
    let Load::Failed(message) = app.list.pane.listing()?.load() else {
        return None;
    };
    let look = &app.look;
    Some(
        col().w_full().px(look.space.lg).py(look.space.lg).child(
            text(message.clone())
                .font(look.kit.ty.ui)
                .color(look.kit.c.text_muted),
        ),
    )
}

fn source(app: &App) -> (&dyn EntrySource, &CollectionState) {
    let pane = &app.list.pane;
    (pane.source(), pane.collection())
}

/// The collection under the table: what the host sets on it, before the table adds its own.
fn collection(app: &App) -> Collection<App> {
    let collection = Collection::new(
        VIEW,
        source,
        |app: &mut App| app.list.pane.parts_mut(),
        |app: &App| &app.look,
        |app: &App| app.list.pane.version(),
    )
    .name(title(app))
    .description(app.list.labels.list.clone())
    .settings(app.list.keys.clone())
    .settings(PointerSettings::default().modes(app.list.clicks))
    .describe(Describe::pure(|app: &App, row| {
        let listing = app.list.pane.listing()?;
        let description = listing.description(row.index, &app.list.words)?;
        Some(description.to_owned())
    }))
    .empty(failure)
    .on_intent(intents::heard)
    .on_cursor(|app: &mut App, _, tx| intents::moved(app, tx))
    .on_selection(intents::moved);
    // The editor answers in this table only while its edit names it.
    collection.rename(
        |app: &App| app.rename.state.renaming_in(VIEW),
        rename::editor(app).slot(),
    )
}

pub(super) fn table(app: &App) -> ListTable<App> {
    let columns = vec![TableColumn::primary(name_id().as_str(), app.list.labels.name.clone())
        // The Name column's default width and minimum (`src/ui/browser_modes/list_columns.rs`).
        .width(160.0)
        .min(160.0)];
    ListTable::new(
        VIEW,
        columns,
        |app: &App| ListParts {
            source: app.list.pane.source(),
            collection: app.list.pane.collection(),
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
    table(app).collection(app, collection(app))
}

/// The view: a group named by the folder, described as the List view, around the table.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let table = table(app).build(app, collection(app), frame.layer);
    Some(
        col()
            .key("list-pane")
            .role("group")
            .label(title(app))
            .description(app.list.labels.pane.clone())
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(table),
    )
}
