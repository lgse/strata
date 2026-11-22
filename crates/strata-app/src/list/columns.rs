// SPDX-License-Identifier: MIT

//! The List table's five columns, as GTK's List view has them
//! (`src/ui/browser_modes/list_columns.rs` and `list_headings.rs`): Name, Mode, Size, Type and
//! Modified, with GTK's default widths and minimums, the order and visibility saved as
//! `list_column_order` and `list_hidden_columns`, and the widths as `browser_list_columns`.
//! GTK's optional Duration, Dimensions and Created columns (`main`'s #2603) are not built
//! here yet, so they are neither shown nor sized: [`SHOWN`] holds the five. What GTK saved for
//! them, their widths, places and hidden state, is kept as it was when this table saves.
//! Cells read the row's facts ([`crate::details`]), as GTK's `list_metadata_label` shows
//! them: left-aligned, in the text colour. The Type cell is read only for the rows drawn, as
//! GTK binds it.

use std::hash::{DefaultHasher, Hash, Hasher};

use strata_components::{
    collection::element::RowCx,
    list_table::{
        Cell, Column as TableColumn, ColumnId, ColumnLayout, ColumnWidths, LayoutChange,
        ListTableLabels,
    },
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
        ListColumnId::Duration => "duration",
        ListColumnId::Dimensions => "dimensions",
        ListColumnId::Created => "created",
    })
}

/// The columns this table builds, in `ListColumnId`'s order.
const SHOWN: [ListColumnId; 5] = [
    ListColumnId::Name,
    ListColumnId::Mode,
    ListColumnId::Size,
    ListColumnId::Type,
    ListColumnId::Modified,
];

fn is_shown(column: ListColumnId) -> bool {
    SHOWN.contains(&column)
}

fn saved(id: &ColumnId) -> Option<ListColumnId> {
    ListColumnId::ALL
        .into_iter()
        .find(|column| self::id(*column) == *id)
}

/// GTK's default widths and minimums, in the order of [`SHOWN`]
/// (`LIST_COLUMN_WIDTHS` and `LIST_COLUMN_MIN_WIDTHS`).
const WIDTHS: [f32; 5] = [160.0, 160.0, 90.0, 120.0, 150.0];
const MINIMUMS: [f32; 5] = [160.0, 80.0, 70.0, 80.0, 110.0];

/// The headings, translated once: the language changes only on restart.
pub(super) struct Headings([String; 5]);

/// The columns menu's and its button's words, translated (`column_options.rs` and
/// `append_columns_menu` in `src/ui/browser_modes`).
pub(super) fn menu_labels() -> ListTableLabels {
    let mut labels = ListTableLabels::default();
    labels.button = tr("Columns");
    labels.heading = tr("COLUMNS");
    labels.fixed = Some(tr("The Name column cannot be hidden"));
    labels.reset = tr("Reset to default columns");
    labels
}

impl Headings {
    pub(super) fn translated() -> Self {
        Self(SHOWN.map(|column| tr(column.title())))
    }
}

fn row<'a>(app: &'a App, rcx: &RowCx<'_>) -> Option<&'a Row> {
    app.list.single.pane.listing()?.row_at(rcx.index)
}

fn mode(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::mode_cell(row.facts()));
    Cell::text(text.unwrap_or_default())
}

fn size(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::size_cell(row.is_folder(), row.facts()));
    Cell::text(text.unwrap_or_default())
}

fn kind(app: &App, rcx: &RowCx<'_>) -> Cell {
    let listing = app.list.single.pane.listing();
    let text = listing.and_then(|listing| {
        let row = listing.row_at(rcx.index)?;
        Some(details::type_cell(
            listing.location(),
            row.entry_kind(),
            row.name(),
        ))
    });
    Cell::text(text.unwrap_or_default())
}

fn modified(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::modified_cell(row.facts()));
    Cell::text(text.unwrap_or_default())
}

/// Moves with what a row's cells read beyond its entry, so only the rows whose facts or date
/// format changed build again; and, while dates are relative, every 30 s, when they read the
/// clock again.
pub(super) fn cells_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    row(app, rcx).map(Row::facts).hash(&mut hasher);
    app.list.date_format.as_str().hash(&mut hasher);
    if app.list.relative_dates {
        app.list.dates.epoch().hash(&mut hasher);
    }
    hasher.finish()
}

/// The five columns in their default order. Name, Size, Type and Modified sort, as GTK's,
/// through A10's `sort::headings`; Mode never.
pub(super) fn columns(headings: &Headings) -> Vec<TableColumn<App>> {
    let [name, mode_title, size_title, kind_title, modified_title] = &headings.0;
    let sized = |column: TableColumn<App>, at: usize| {
        let column = column.width(WIDTHS[at]).min(MINIMUMS[at]);
        if crate::sort::headings::sortable(column.id().as_str()) {
            column.sortable()
        } else {
            column
        }
    };
    vec![
        sized(TableColumn::primary("name", name.clone()), 0),
        sized(TableColumn::new("mode", mode_title.clone(), mode), 1),
        sized(TableColumn::new("size", size_title.clone(), size), 2),
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
    let defaults: Vec<ColumnId> = SHOWN.into_iter().map(id).collect();
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
        .as_ref()
        .filter(|saved| saved.is_valid())
        .map_or_else(ColumnWidths::new, widths_of)
}

pub(super) fn chooser_widths(settings: &Preferences) -> ColumnWidths {
    settings
        .chooser_list_columns
        .as_ref()
        .filter(|saved| saved.is_valid())
        .map_or_else(ColumnWidths::new, widths_of)
}

fn widths_of(saved: &ListColumns) -> ColumnWidths {
    #[expect(clippy::cast_precision_loss, reason = "a width in points")]
    let points = |width: i32| width as f32;
    SHOWN
        .into_iter()
        .filter_map(|column| Some((column, saved.width(column)?)))
        .fold(ColumnWidths::new(), |widths, (column, width)| {
            widths.with(&id(column), points(width))
        })
}

/// The widths as GTK's `remember` saves them: all four others, in whole points, and Name only
/// once resized by hand.
fn saved_widths(widths: &ColumnWidths) -> Vec<(ListColumnId, i32)> {
    #[expect(clippy::cast_possible_truncation, reason = "a width in points")]
    let width = |column: ListColumnId, at: usize| {
        widths.get(&id(column)).unwrap_or(WIDTHS[at]).round() as i32
    };
    SHOWN
        .into_iter()
        .enumerate()
        .filter(|(_, column)| {
            *column != ListColumnId::Name || widths.get(&id(ListColumnId::Name)).is_some()
        })
        .map(|(at, column)| (column, width(column, at)))
        .collect()
}

/// This table's widths, with the widths `saved` has for the columns it does not build.
fn merged_widths(saved: Option<&ListColumns>, widths: &ColumnWidths) -> ListColumns {
    let kept = ListColumnId::ALL
        .into_iter()
        .filter(|column| !is_shown(*column))
        .filter_map(|column| Some((column, saved?.width(column)?)));
    saved_widths(widths).into_iter().chain(kept).collect()
}

/// `order`'s columns in the places this table's columns had in `saved`, and the columns it does
/// not build left where they were.
fn merged_order(saved: &[ListColumnId], order: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut ours = order.iter().copied().filter(|column| is_shown(*column));
    let merged: Vec<ListColumnId> = sanitize_list_column_order(saved)
        .into_iter()
        .map(|column| {
            if is_shown(column) {
                ours.next().unwrap_or(column)
            } else {
                column
            }
        })
        .collect();
    sanitize_list_column_order(&merged)
}

/// `hidden`, with the columns this table does not build hidden as `saved` has them.
fn merged_hidden(saved: &[ListColumnId], hidden: &[ListColumnId]) -> Vec<ListColumnId> {
    let kept = saved.iter().copied().filter(|column| !is_shown(*column));
    sanitize_list_hidden_columns(&hidden.iter().copied().chain(kept).collect::<Vec<_>>())
}

/// The person reordered, showed, hid or reset the columns: the settings save it through the
/// rules GTK's setters apply (`set_list_column_order`, `set_list_hidden_columns`), which
/// `strata_desktop::settings` holds, and the window keeps what a restart would read back.
pub(super) fn keep_layout(app: &mut App, change: LayoutChange) {
    let order: Vec<ListColumnId> = change.layout.order().iter().filter_map(saved).collect();
    let hidden: Vec<ListColumnId> = change.layout.hidden().filter_map(saved).collect();
    let settings = shell::preferences(app);
    let order = merged_order(&settings.list_column_order, &order);
    let hidden = merged_hidden(&settings.list_hidden_columns, &hidden);
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
    let chooser = app.browser.is_chooser_mode();
    let settings = shell::preferences(app);
    let before = if chooser {
        settings.chooser_list_columns.as_ref()
    } else {
        settings.browser_list_columns.as_ref()
    };
    let saved = Some(merged_widths(before, &widths)).filter(ListColumns::is_valid);
    app.list.widths = saved.as_ref().map_or_else(ColumnWidths::new, widths_of);
    shell::saved(app).change(|settings| {
        if chooser {
            settings.chooser_list_columns = saved;
        } else {
            settings.browser_list_columns = saved;
        }
    });
}
