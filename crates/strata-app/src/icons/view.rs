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

use super::{Icons, VIEW};
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
    .on_wheel(single::wheeled::<Icons>)
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

/// The view: the pane's group around the grid.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let grid = grid().scale(frame.cx.env().scale).build(app);
    Some(single::group::<Icons>(app, grid))
}
