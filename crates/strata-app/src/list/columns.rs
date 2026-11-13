// SPDX-License-Identifier: MIT

//! The List table's five columns, as GTK's List view has them
//! (`src/ui/browser_modes/list_columns.rs` and `list_headings.rs`): Name, Mode, Size, Type and
//! Modified, with GTK's default widths and minimums, the order and visibility saved as
//! `list_column_order` and `list_hidden_columns`, and the widths as `browser_list_columns`.
//! Cells read the row's facts ([`crate::details`]); the Type cell is read only for the rows
//! drawn, as GTK binds it.

use std::hash::{DefaultHasher, Hash, Hasher};

use rune::Align;
use strata_components::{
    collection::element::RowCx,
    list_table::{Cell, Column as TableColumn, ColumnId, ColumnLayout, ColumnWidths, LayoutChange},
};
use strata_core::i18n::tr;
use strata_desktop::settings::{
    ListColumnId, ListColumns, Preferences, sanitize_list_column_order,
    sanitize_list_hidden_columns,
};

use crate::{App, details, listing::Row, shell};

/// The id a column is saved by (`ListColumnId`'s serde name).
fn id(column: ListColumnId) -> ColumnId {
    ColumnId::new(match column {
        ListColumnId::Name => "name",
        ListColumnId::Mode => "mode",
        ListColumnId::Size => "size",
        ListColumnId::Type => "type",
        ListColumnId::Modified => "modified",
    })
}

fn saved(id: &ColumnId) -> Option<ListColumnId> {
    ListColumnId::ALL
        .into_iter()
        .find(|column| self::id(*column) == *id)
}

/// GTK's default widths and minimums, in the order of `ListColumnId::ALL`
/// (`LIST_COLUMN_WIDTHS` and `LIST_COLUMN_MIN_WIDTHS`).
const WIDTHS: [f32; 5] = [160.0, 160.0, 90.0, 120.0, 150.0];
const MINIMUMS: [f32; 5] = [160.0, 80.0, 70.0, 80.0, 110.0];

/// The headings, translated once: the language changes only on restart.
pub(super) struct Headings([String; 5]);

impl Headings {
    pub(super) fn translated() -> Self {
        Self(ListColumnId::ALL.map(|column| tr(column.title())))
    }
}

fn row<'a>(app: &'a App, rcx: &RowCx<'_>) -> Option<&'a Row> {
    app.list.single.pane.listing()?.row_of(rcx.entry.id)
}

fn mode(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::mode_cell(row.facts()));
    Cell::text(text.unwrap_or_default()).mono().muted()
}

fn size(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::size_cell(row.is_folder(), row.facts()));
    Cell::text(text.unwrap_or_default()).muted()
}

fn kind(app: &App, rcx: &RowCx<'_>) -> Cell {
    let listing = app.list.single.pane.listing();
    let text = listing.and_then(|listing| {
        let row = listing.row_of(rcx.entry.id)?;
        Some(details::type_cell(
            listing.location(),
            row.entry_kind(),
            row.name(),
        ))
    });
    Cell::text(text.unwrap_or_default()).muted()
}

fn modified(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::modified_cell(row.facts()));
    Cell::text(text.unwrap_or_default()).muted()
}

/// Moves with what a row's cells read beyond its entry, so only the rows whose facts changed
/// build again.
pub(super) fn cells_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    row(app, rcx).map(Row::facts).hash(&mut hasher);
    hasher.finish()
}

/// The five columns in their default order. None is sortable yet: a sortable heading is a
/// control that sorts when pressed, and nothing sorts until A10 gives the table `on_sort`
/// (Name, Size, Type and Modified, as GTK's; Mode never).
pub(super) fn columns(headings: &Headings) -> Vec<TableColumn<App>> {
    let [name, mode_title, size_title, kind_title, modified_title] = &headings.0;
    let sized = |column: TableColumn<App>, at: usize| column.width(WIDTHS[at]).min(MINIMUMS[at]);
    vec![
        sized(TableColumn::primary("name", name.clone()), 0),
        sized(TableColumn::new("mode", mode_title.clone(), mode), 1),
        sized(TableColumn::new("size", size_title.clone(), size), 2).align(Align::End),
        sized(TableColumn::new("type", kind_title.clone(), kind), 3),
        sized(
            TableColumn::new("modified", modified_title.clone(), modified),
            4,
        ),
    ]
}

/// The saved order and visibility, made whole as GTK loads them (`ColumnLayout::normalize`).
pub(super) fn layout(settings: &Preferences) -> ColumnLayout {
    layout_of(&settings.list_column_order, &settings.list_hidden_columns)
}

fn layout_of(order: &[ListColumnId], hidden: &[ListColumnId]) -> ColumnLayout {
    let defaults: Vec<ColumnId> = ListColumnId::ALL.into_iter().map(id).collect();
    let order: Vec<ColumnId> = order.iter().map(|column| id(*column)).collect();
    let hidden: Vec<ColumnId> = hidden.iter().map(|column| id(*column)).collect();
    ColumnLayout::normalize(
        &defaults,
        Some(&id(ListColumnId::Name)),
        order.iter().map(ColumnId::as_str),
        hidden.iter().map(ColumnId::as_str),
    )
}

/// The saved widths (`browser_list_columns`), in base points, the unit C4 keeps and GTK saves
/// (`ListColumnLayout::remember` divides by the interface scale, root font px over 13): Name
/// only once resized by hand.
pub(super) fn widths(settings: &Preferences) -> ColumnWidths {
    settings
        .browser_list_columns
        .filter(ListColumns::is_valid)
        .map_or_else(ColumnWidths::new, |saved| widths_of(&saved))
}

fn widths_of(saved: &ListColumns) -> ColumnWidths {
    #[expect(clippy::cast_precision_loss, reason = "a width in points")]
    let points = |width: i32| width as f32;
    let widths = ColumnWidths::new()
        .with(&id(ListColumnId::Mode), points(saved.mode))
        .with(&id(ListColumnId::Size), points(saved.size))
        .with(&id(ListColumnId::Type), points(saved.kind))
        .with(&id(ListColumnId::Modified), points(saved.modified));
    match saved.name {
        Some(name) => widths.with(&id(ListColumnId::Name), points(name)),
        None => widths,
    }
}

/// The widths as GTK's `remember` saves them: all four others, in whole points, and Name only
/// once resized by hand.
fn saved_widths(widths: &ColumnWidths) -> ListColumns {
    #[expect(clippy::cast_possible_truncation, reason = "a width in points")]
    let width = |column: ListColumnId, at: usize| {
        widths.get(&id(column)).unwrap_or(WIDTHS[at]).round() as i32
    };
    ListColumns {
        name: widths
            .get(&id(ListColumnId::Name))
            .map(|_| width(ListColumnId::Name, 0)),
        mode: width(ListColumnId::Mode, 1),
        size: width(ListColumnId::Size, 2),
        kind: width(ListColumnId::Type, 3),
        modified: width(ListColumnId::Modified, 4),
    }
}

/// The person reordered, showed, hid or reset the columns: the settings save it through the
/// rules GTK's setters apply (`set_list_column_order`, `set_list_hidden_columns`), which
/// `strata_desktop::settings` holds, and the window keeps what a restart would read back.
pub(super) fn keep_layout(app: &mut App, change: LayoutChange) {
    let order: Vec<ListColumnId> = change.layout.order().iter().filter_map(saved).collect();
    let hidden: Vec<ListColumnId> = change.layout.hidden().filter_map(saved).collect();
    let order = sanitize_list_column_order(&order);
    let hidden = sanitize_list_hidden_columns(&hidden);
    app.list.layout = layout_of(&order, &hidden);
    shell::saved(app).change(|settings| {
        settings.list_column_order = order;
        settings.list_hidden_columns = hidden;
    });
}

/// The person resized a column: the settings save every width, as GTK saves
/// `browser_list_columns`, keeping only valid ones (`set_browser_list_columns`), and the
/// window keeps what a restart would read back.
pub(super) fn keep_widths(app: &mut App, widths: ColumnWidths) {
    let saved = Some(saved_widths(&widths)).filter(ListColumns::is_valid);
    app.list.widths = saved.as_ref().map_or_else(ColumnWidths::new, widths_of);
    shell::saved(app).change(|settings| settings.browser_list_columns = saved);
}
