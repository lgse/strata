// SPDX-License-Identifier: MIT

//! The Icons grid over the pane's listing and collection.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use rune::El;
use strata_components::{
    collection::element::RowCx,
    icons_grid::{IconsGrid, IconsParts},
};
use strata_core::util::DateFormat;

use super::{Icons, VIEW, size};
use crate::{App, details, listing::Row, shell, single, slots::Frame};

pub(crate) fn grid() -> IconsGrid<App> {
    IconsGrid::new(
        VIEW,
        |app: &App| crate::peek::rows(single::collection::<Icons>(app)),
        |app: &App| IconsParts {
            source: app.icons.single.pane.source(),
            collection: app.icons.single.pane.collection(),
            grid: &app.icons.grid,
            size: app.icons.size,
            listing: app.icons.single.pane.thumb_version(),
            look: &app.look,
            thumbs: app.icons.thumbs.id().map(|id| (shell::thumbnails(app), id)),
        },
        |app: &mut App| &mut app.icons.grid,
    )
    .thumbs_mut(|app: &mut App| {
        let id = app.icons.thumbs.id()?;
        let source = app.icons.single.pane.source();
        Some((app.window.thumbnails_mut(), id, source))
    })
    .details(details_version, details_text)
    .tooltip(tooltip_version, tooltip_text)
    .badge(
        crate::file_providers::badge_version,
        crate::file_providers::badge_pixels,
    )
    .on_visible(|app: &mut App, range| {
        // Recorded only: the part asks for the fills once a poll.
        app.icons.fills.see(range);
        false
    })
    .on_wheel(single::wheeled::<Icons>)
    .on_size(size::resized)
}

fn row<'a>(app: &'a App, rcx: &RowCx<'_>) -> Option<&'a Row> {
    app.icons.single.pane.listing()?.row_at(rcx.index)
}

fn details_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    row(app, rcx).map(Row::facts).hash(&mut hasher);
    hasher.finish()
}

fn details_text(app: &App, rcx: &RowCx<'_>) -> Option<Arc<str>> {
    let row = row(app, rcx)?;
    details::icons_line(row.is_folder(), row.facts()).map(Arc::from)
}

fn tooltip_version_for(app: &App, row: Option<&Row>) -> u64 {
    let mut hasher = DefaultHasher::new();
    row.and_then(|row| row.facts().modified).hash(&mut hasher);
    app.icons.date_format.as_str().hash(&mut hasher);
    if app.icons.date_format == DateFormat::Relative {
        app.icons.dates.epoch().hash(&mut hasher);
    }
    hasher.finish()
}

fn tooltip_text_for(row: Option<&Row>, rcx: &RowCx<'_>, truncated: bool) -> Option<String> {
    details::entry_tooltip(rcx.entry.name, row.map(Row::facts), true, || truncated)
}

fn tooltip_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    tooltip_version_for(app, row(app, rcx))
}

fn tooltip_text(app: &App, rcx: &RowCx<'_>, truncated: bool) -> Option<String> {
    tooltip_text_for(row(app, rcx), rcx, truncated)
}

fn hit_row<'a>(app: &'a App, rcx: &RowCx<'_>) -> Option<&'a Row> {
    app.filter.pane.hits()?.row_at(rcx.index)
}

fn results_tooltip_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    tooltip_version_for(app, hit_row(app, rcx))
}

fn results_tooltip_text(app: &App, rcx: &RowCx<'_>, truncated: bool) -> Option<String> {
    tooltip_text_for(hit_row(app, rcx), rcx, truncated)
}

pub(crate) fn results() -> IconsGrid<App> {
    IconsGrid::new(
        VIEW,
        single::results::<Icons>,
        |app: &App| IconsParts {
            source: app.filter.pane.source(),
            collection: app.filter.pane.collection(),
            grid: &app.filter.pane.grid,
            size: app.icons.size,
            listing: app.filter.pane.version(),
            look: &app.look,
            thumbs: None,
        },
        |app: &mut App| &mut app.filter.pane.grid,
    )
    .details(
        |app: &App, rcx: &RowCx<'_>| {
            let mut hasher = DefaultHasher::new();
            crate::filter::pane::path_of(app, rcx.entry.id).hash(&mut hasher);
            hasher.finish()
        },
        |app: &App, rcx: &RowCx<'_>| crate::filter::pane::path_of(app, rcx.entry.id).map(Arc::from),
    )
    .tooltip(results_tooltip_version, results_tooltip_text)
    .badge(
        crate::file_providers::badge_version,
        crate::file_providers::badge_pixels,
    )
    .on_size(size::resized)
}

/// The view: the pane's group around the grid, or around a filter's results.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let scale = frame.cx.env().scale;
    app.icons.single.set_scale(scale);
    let body = match app.filter.pane.shows() {
        true => results().scale(scale).build(app),
        false => grid().scale(scale).build(app),
    };
    Some(single::group::<Icons>(app, body, None))
}
