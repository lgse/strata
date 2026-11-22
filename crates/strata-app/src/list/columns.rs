// SPDX-License-Identifier: MIT

//! The List table's eight columns, as GTK's List view has them
//! (`src/ui/browser_modes/list_columns.rs` and `list_headings.rs`): Name, Mode, Size, Type and
//! Modified shown by default, and optional Duration, Dimensions and Created hidden by default,
//! with GTK's default widths and minimums, the order and visibility saved as
//! `list_column_order`, `list_hidden_columns` and `list_shown_columns`, and the widths as
//! `browser_list_columns` / `chooser_list_columns`.
//! Cells read the row's facts ([`crate::details`]), as GTK's `list_metadata_label` shows
//! them: left-aligned, in the text colour. The Type cell is read only for the rows drawn, as
//! GTK binds it.

use std::{
    collections::HashSet,
    hash::{DefaultHasher, Hash, Hasher},
};

use strata_components::{
    collection::element::RowCx,
    list_table::{
        Cell, Column as TableColumn, ColumnId, ColumnLayout, ColumnWidths, LayoutChange, LayoutWhy,
        ListTableLabels,
    },
};
use strata_core::{i18n::tr, model::SortKey};
use strata_desktop::settings::{
    ListColumnId, ListColumns, Preferences, default_list_column_order, sanitize_list_column_order,
    sanitize_list_hidden_columns,
};

use crate::{App, details, fills::Details, listing::Row, shell};

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

fn saved(id: &ColumnId) -> Option<ListColumnId> {
    ListColumnId::ALL
        .into_iter()
        .find(|column| self::id(*column) == *id)
}

/// Whether `column` is visible under `settings` (`is_list_column_visible` in
/// `src/ui/preferences/layout_settings.rs`): Name is always visible, optional columns require
/// an opt-in in `list_shown_columns`, and standard columns stay visible unless listed in
/// `list_hidden_columns`.
pub(crate) fn is_visible(settings: &Preferences, column: ListColumnId) -> bool {
    if column == ListColumnId::Name {
        return true;
    }
    if column.definition().optional {
        settings.list_shown_columns.contains(&column)
    } else {
        !settings.list_hidden_columns.contains(&column)
    }
}

/// The metadata detail level the List view asks for: media details for all rows while Duration
/// or Dimensions is visible (`ListRow::has_media_column` in `list_factory.rs`), otherwise
/// folder item counts only.
pub(super) fn fill_details(layout: &ColumnLayout) -> Details {
    let media = ListColumnId::ALL.into_iter().any(|column| {
        column
            .definition()
            .sort_key
            .is_some_and(SortKey::uses_media_metadata)
            && layout.is_visible(&id(column))
    });
    if media {
        Details::All
    } else {
        Details::Folders
    }
}

/// The headings, translated once: the language changes only on restart.
pub(super) struct Headings([String; ListColumnId::ALL.len()]);

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
        Self(ListColumnId::ALL.map(|column| tr(column.title())))
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

fn duration(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::duration_cell(row.is_folder(), row.facts()));
    Cell::text(text.unwrap_or_default())
}

fn dimensions(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::dimensions_cell(row.is_folder(), row.facts()));
    Cell::text(text.unwrap_or_default())
}

fn created(app: &App, rcx: &RowCx<'_>) -> Cell {
    let text = row(app, rcx).map(|row| details::created_cell(row.facts()));
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

fn build_column(column: ListColumnId, title: String) -> TableColumn<App> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "column widths are small point counts"
    )]
    let (width, min_width) = {
        let def = column.definition();
        (def.width as f32, def.min_width as f32)
    };
    let table_column = match column {
        ListColumnId::Name => TableColumn::primary("name", title),
        ListColumnId::Mode => TableColumn::new("mode", title, mode),
        ListColumnId::Size => TableColumn::new("size", title, size),
        ListColumnId::Type => TableColumn::new("type", title, kind),
        ListColumnId::Modified => TableColumn::new("modified", title, modified),
        ListColumnId::Duration => TableColumn::new("duration", title, duration),
        ListColumnId::Dimensions => TableColumn::new("dimensions", title, dimensions),
        ListColumnId::Created => TableColumn::new("created", title, created),
    }
    .width(width)
    .min(min_width);
    if crate::sort::headings::sortable(table_column.id().as_str()) {
        table_column.sortable()
    } else {
        table_column
    }
}

/// All eight columns in their default order. Every column except Mode sorts through
/// `sort::headings`.
pub(super) fn columns(headings: &Headings) -> Vec<TableColumn<App>> {
    ListColumnId::ALL
        .into_iter()
        .zip(headings.0.iter())
        .map(|(column, title)| build_column(column, title.clone()))
        .collect()
}

/// The saved order and visibility, made whole as GTK loads them (`ColumnLayout::normalize`).
pub(super) fn layout(settings: &Preferences) -> ColumnLayout {
    layout_of(
        &settings.list_column_order,
        &settings.list_hidden_columns,
        &settings.list_shown_columns,
    )
}

fn layout_of(
    order: &[ListColumnId],
    hidden: &[ListColumnId],
    shown: &[ListColumnId],
) -> ColumnLayout {
    let defaults: Vec<ColumnId> = ListColumnId::ALL.into_iter().map(id).collect();
    let order: Vec<ColumnId> = order.iter().map(|column| id(*column)).collect();
    let hidden_ids: Vec<ColumnId> = ListColumnId::ALL
        .into_iter()
        .filter(|&column| {
            column != ListColumnId::Name
                && if column.definition().optional {
                    !shown.contains(&column)
                } else {
                    hidden.contains(&column)
                }
        })
        .map(id)
        .collect();
    ColumnLayout::normalize(
        &defaults,
        Some(&id(ListColumnId::Name)),
        order.iter().map(ColumnId::as_str),
        hidden_ids.iter().map(ColumnId::as_str),
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
    ListColumnId::ALL
        .into_iter()
        .filter_map(|column| Some((column, saved.width(column)?)))
        .fold(ColumnWidths::new(), |widths, (column, width)| {
            widths.with(&id(column), points(width))
        })
}

/// The widths as GTK's `remember` saves them: every metadata column in whole points, and Name
/// only once resized by hand.
fn saved_widths(widths: &ColumnWidths) -> ListColumns {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "column widths are small point counts"
    )]
    let width = |column: ListColumnId| {
        widths
            .get(&id(column))
            .unwrap_or(column.definition().width as f32)
            .round() as i32
    };
    ListColumnId::ALL
        .into_iter()
        .filter(|&column| {
            column != ListColumnId::Name || widths.get(&id(ListColumnId::Name)).is_some()
        })
        .map(|column| (column, width(column)))
        .collect()
}

/// Updates a saved column-visibility list (`list_hidden_columns` or `list_shown_columns`),
/// preserving the saved order of retained entries and appending newly added ones.
fn updated_set(
    saved: &[ListColumnId],
    belongs: impl Fn(ListColumnId) -> bool,
) -> Vec<ListColumnId> {
    let mut seen = HashSet::new();
    saved
        .iter()
        .copied()
        .chain(ListColumnId::ALL)
        .filter(|&column| belongs(column) && seen.insert(column))
        .collect()
}

/// The person reordered, showed, hid or reset the columns: the settings save it through the
/// rules GTK's setters apply (`set_list_column_order`, `set_list_column_visible`,
/// `reset_list_columns`), and the window keeps what a restart would read back.
pub(super) fn keep_layout(app: &mut App, change: LayoutChange) {
    let (order, hidden, shown) = if change.why == LayoutWhy::Reset {
        (default_list_column_order(), Vec::new(), Vec::new())
    } else {
        let settings = shell::preferences(app);
        let order = sanitize_list_column_order(
            &change
                .layout
                .order()
                .iter()
                .filter_map(saved)
                .collect::<Vec<_>>(),
        );
        let hidden =
            sanitize_list_hidden_columns(&updated_set(&settings.list_hidden_columns, |column| {
                column != ListColumnId::Name
                    && !column.definition().optional
                    && !change.layout.is_visible(&id(column))
            }));
        let shown = updated_set(&settings.list_shown_columns, |column| {
            column.definition().optional && change.layout.is_visible(&id(column))
        });
        (order, hidden, shown)
    };
    app.list.layout = layout_of(&order, &hidden, &shown);
    app.list.fills.set_details(fill_details(&app.list.layout));
    shell::saved(app).change(|settings| {
        settings.list_column_order = order;
        settings.list_hidden_columns = hidden;
        settings.list_shown_columns = shown;
    });
}

/// The person resized a column: the settings save every width, as GTK saves
/// `browser_list_columns` / `chooser_list_columns`, keeping only valid ones, and the window
/// keeps what a restart would read back.
pub(super) fn keep_widths(app: &mut App, widths: ColumnWidths) {
    let chooser = app.browser.is_chooser_mode();
    let saved = Some(saved_widths(&widths)).filter(ListColumns::is_valid);
    app.list.widths = saved.as_ref().map_or_else(ColumnWidths::new, widths_of);
    shell::saved(app).change(|settings| {
        if chooser {
            settings.chooser_list_columns = saved;
        } else {
            settings.browser_list_columns = saved;
        }
    });
}
