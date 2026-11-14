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

use super::{Icons, VIEW, size};
use crate::{App, details, listing::Row, shell, single, slots::Frame};

pub(super) fn grid() -> IconsGrid<App> {
    IconsGrid::new(
        VIEW,
        single::collection::<Icons>,
        |app: &App| IconsParts {
            source: app.icons.single.pane.source(),
            collection: app.icons.single.pane.collection(),
            grid: &app.icons.grid,
            size: app.icons.size,
            listing: app.icons.single.pane.version(),
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
    .on_visible(|app: &mut App, range| {
        // Recorded only: the part asks for the fills once a poll.
        app.icons.fills.see(range);
        false
    })
    .on_wheel(single::wheeled::<Icons>)
    .on_size(size::resized)
}

fn row<'a>(app: &'a App, rcx: &RowCx<'_>) -> Option<&'a Row> {
    app.icons.single.pane.listing()?.row_of(rcx.entry.id)
}

/// Moves with what a tile's details line reads, so only the tiles whose facts changed build
/// again.
fn details_version(app: &App, rcx: &RowCx<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    row(app, rcx).map(Row::facts).hash(&mut hasher);
    hasher.finish()
}

fn details_text(app: &App, rcx: &RowCx<'_>) -> Option<Arc<str>> {
    let row = row(app, rcx)?;
    details::icons_line(row.is_folder(), row.facts()).map(Arc::from)
}

/// A filter's results in the grid's place (A10's [`crate::filter::pane`]): tiles as the
/// grid's, at its size, each with its path under the folder as its details line while the
/// filter searches the subfolders (GTK's `ResultKind::Icons`).
pub(super) fn results() -> IconsGrid<App> {
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
    .on_size(size::resized)
}

/// The view: the pane's group around the grid, or around a filter's results.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let scale = frame.cx.env().scale;
    let body = match app.filter.pane.shows() {
        true => results().scale(scale).build(app),
        false => grid().scale(scale).build(app),
    };
    Some(single::group::<Icons>(app, body, None))
}
