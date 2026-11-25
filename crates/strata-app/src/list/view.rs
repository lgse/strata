// SPDX-License-Identifier: MIT

//! The List table over the pane's listing and collection.

use std::hash::{DefaultHasher, Hash, Hasher};

use rune::{El, col, text};
use strata_components::{
    collection::element::{Collection, LABEL, Roles, RowCx, Slot},
    list_table::{ListParts, ListTable},
};
use strata_kit::look::Look;

use super::{List, VIEW, columns};
use crate::{App, details, single, slots::Frame};

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
    .tooltip(columns::row_tooltip)
    .on_layout(|app: &mut App, change, _| columns::keep_layout(app, change))
    .on_widths(|app: &mut App, widths, _| columns::keep_widths(app, widths))
    .on_sort(|app: &mut App, intent, _| crate::sort::headings::pressed(app, intent))
}

/// The pane's collection, with the rows the viewport shows recorded for the fills.
fn base(app: &App) -> Collection<App> {
    crate::peek::rows(single::collection::<List>(app))
        .icon(app.list.thumbs.icon(&app.look))
        .on_visible(|app: &mut App, range| {
            // Recorded only: the part asks for the fills and syncs the thumbnails once a poll.
            app.list.fills.see(range.clone());
            app.list.thumbs.see(range);
            false
        })
}

/// The collection with the table's parts: what the poll ticks.
pub(crate) fn ticked(app: &App) -> Collection<App> {
    table(app).collection(app, base(app))
}

pub(crate) fn results(app: &App) -> Collection<App> {
    let paths = app.filter.pane.recursive;
    let feed = app.filter.pane.list_feed().clone();
    single::results::<List>(app)
        .roles(Roles::LIST)
        .icon(app.list.thumbs.icon(&app.look))
        .on_visible(|app: &mut App, range| {
            app.list.thumbs.see(range);
            false
        })
        .row_height(move |_: &App, look: &Look| match paths {
            true => (look.row.list + look.kit.ty.small.line_height + 2.0).round(),
            false => look.row.list,
        })
        .row_gap_y(1.0)
        .on_scroll(move |pos| feed.set(pos))
        .label(Slot::new(results_label_version, results_label))
}

fn results_label_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    let modified = app
        .filter
        .pane
        .hits()
        .and_then(|hits| hits.row_at(rcx.index))
        .and_then(|row| row.facts().modified);
    modified.hash(&mut hasher);
    app.list.date_format.as_str().hash(&mut hasher);
    if app.list.relative_dates {
        app.list.dates.epoch().hash(&mut hasher);
    }
    hasher.finish()
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
    let spans = rcx.state.marks.name_matches.spans(rcx.look);
    let mut name = line(
        rcx.entry.name.to_owned(),
        LABEL,
        kit.ty.ui_compact,
        kit.c.text_base,
    )
    .spans(spans);
    let can_tip = !rcx.state.renaming
        && !rcx.state.marks.dragging
        && !rcx.state.marks.peek_anchor
        && !rcx.entry.name.is_empty();
    let date_tip = can_tip
        .then(|| {
            let row = app.filter.pane.hits()?.row_at(rcx.index)?;
            let date = details::modified_tooltip_date(row.facts())?;
            details::entry_tooltip_text(rcx.entry.name, false, Some(&date))
        })
        .flatten();
    if can_tip && date_tip.is_none() {
        name = name.help_when_truncated(rcx.entry.name.to_owned());
    }
    let mut label = col().shrink().min_w(0.0).child(name);
    if let Some(path) = crate::filter::pane::path_of(app, rcx.entry.id) {
        let path = line(path, LABEL, kit.ty.small, kit.c.text_muted);
        label = label.child(col().key("path").min_w(0.0).child(path));
    }
    if let Some(tip) = date_tip {
        label = label.help(tip);
    }
    label
}

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let scale = frame.cx.env().scale;
    app.list.single.set_scale(scale);
    app.list.thumbs.set_scale(scale);
    let columns = columns_button(app, frame.layer);
    let body = match app.filter.pane.shows() {
        true => results(app).build(app),
        false => table(app).build(app, base(app), frame.layer),
    };
    Some(single::group::<List>(app, body, Some(columns)))
}

fn columns_button(app: &App, layer: &mut Vec<El>) -> El {
    table(app).columns_button(app, layer)
}
