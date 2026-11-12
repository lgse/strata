// SPDX-License-Identifier: MIT

//! The Icons grid over the pane's listing and collection.

use rune::El;
use strata_components::icons_grid::{IconsGrid, IconsParts};

use super::{Icons, VIEW};
use crate::{App, shell, single, slots::Frame};

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
}

/// The view: the pane's group around the grid.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let grid = grid().scale(frame.cx.env().scale).build(app);
    Some(single::group::<Icons>(app, grid))
}
