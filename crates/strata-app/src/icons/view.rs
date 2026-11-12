// SPDX-License-Identifier: MIT

//! The Icons grid over the pane's listing and collection, with Strata's words.

use rune::{El, col, text};
use strata_components::{
    collection::{
        element::{Collection, CollectionState, Describe},
        pointer::PointerSettings,
    },
    contract::EntrySource,
    icons_grid::{IconsGrid, IconsParts},
};
use strata_core::i18n::tr;

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
    /// `ENTRY_CONTAINER_DESCRIPTION` there: what the grid of entries is.
    list: String,
}

impl Labels {
    pub(super) fn translated() -> Self {
        Self {
            pane: tr("Icons view"),
            list: tr("Files"),
        }
    }
}

/// The folder the grid shows, as the pane and the grid are named: `display_name`, as GTK's
/// `describe_pane` and `describe_entry_container` name them.
fn title(app: &App) -> String {
    app.icons
        .pane
        .listing()
        .map(Column::title)
        .unwrap_or_default()
        .to_owned()
}

/// What the grid shows in place of tiles when its folder could not be read: the browser's
/// message.
fn failure(app: &App) -> Option<El> {
    let Load::Failed(message) = app.icons.pane.listing()?.load() else {
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
    let pane = &app.icons.pane;
    (pane.source(), pane.collection())
}

/// The collection under the grid: what the host sets on it, before the grid adds its own.
fn collection(app: &App) -> Collection<App> {
    let collection = Collection::new(
        VIEW,
        source,
        |app: &mut App| app.icons.pane.parts_mut(),
        |app: &App| &app.look,
        |app: &App| app.icons.pane.version(),
    )
    .name(title(app))
    .description(app.icons.labels.list.clone())
    .settings(app.icons.keys.clone())
    .settings(PointerSettings::default().modes(app.icons.clicks))
    .describe(Describe::pure(|app: &App, row| {
        let listing = app.icons.pane.listing()?;
        let description = listing.description(row.index, &app.icons.words)?;
        Some(description.to_owned())
    }))
    .empty(failure)
    .on_intent(intents::heard)
    .on_cursor(|app: &mut App, _, tx| intents::moved(app, tx))
    .on_selection(intents::moved);
    // The editor answers in this grid only while its edit names it.
    collection.rename(
        |app: &App| app.rename.state.renaming_in(VIEW),
        rename::editor(app).slot(),
    )
}

pub(super) fn grid() -> IconsGrid<App> {
    IconsGrid::new(
        VIEW,
        collection,
        |app: &App| IconsParts {
            source: app.icons.pane.source(),
            collection: app.icons.pane.collection(),
            grid: &app.icons.grid,
            size: app.icons.size,
            listing: app.icons.pane.version(),
            look: &app.look,
            thumbs: None,
        },
        |app: &mut App| &mut app.icons.grid,
    )
}

/// The view: a group named by the folder, described as the Icons view, around the grid.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let grid = grid().scale(frame.cx.env().scale).build(app);
    Some(
        col()
            .key("icons-pane")
            .role("group")
            .label(title(app))
            .description(app.icons.labels.pane.clone())
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(grid),
    )
}
