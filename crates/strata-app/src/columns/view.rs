// SPDX-License-Identifier: MIT

//! The Columns view over the adapter's listings and state, with Strata's words.

use std::borrow::Cow;

use rune::El;
use strata_components::{
    collection::{element::Collection, pointer::PointerSettings},
    columns::{ColumnsLabels, ColumnsParts, ColumnsPartsMut, ColumnsView},
};
use strata_core::i18n::tr;

use super::intents;
use crate::{App, commands, filter, pane_header, rename, shell, surface};

/// The view's words, translated once: the language changes only on restart.
pub(super) struct Labels(ColumnsLabels);

impl Labels {
    pub(super) fn translated() -> Self {
        let mut labels = ColumnsLabels::default();
        labels.strip = tr("Columns");
        labels.close = tr("Close this pane");
        labels.loading = tr("Loading");
        labels.width = tr("Column width");
        // `view_name` and `ENTRY_CONTAINER_DESCRIPTION` in `src/ui/accessibility.rs`: the
        // QA harness finds panes and their lists by these.
        labels.pane = tr("Columns view");
        labels.list = tr("Files");
        labels.reveal = rust_i18n::t!("Reveal %{value1} column", value1 = "{name}").into_owned();
        Self(labels)
    }
}

/// What column `depth` says in place of rows while it shows none ([`crate::surface`]), and
/// whether it draws it. A filter's results say what the filter does.
fn status(app: &App, depth: usize) -> Option<(Cow<'_, str>, bool)> {
    if app.columns.listings.hits_at(depth) {
        return filter::status(app).map(|said| (Cow::Owned(said), true));
    }
    let listing = app.columns.listings.columns.get(depth)?;
    app.columns.surface.status(depth, listing)
}

/// What a column draws in place of rows, as GTK's column draws it (`show_empty` and
/// `show_error` in `src/ui/browser/presentation.rs`).
fn shown_status(app: &App, depth: usize) -> Option<El> {
    if app.columns.listings.hits_at(depth) {
        return filter::status_element(app);
    }
    let (said, _) = status(app, depth).filter(|(_, drawn)| *drawn)?;
    let listing = app.columns.listings.columns.get(depth)?;
    let failed = surface::failed(listing, depth);
    Some(surface::drawn(app, &app.columns.surface, &said, failed))
}

pub(crate) fn view(app: &App) -> ColumnsView<App> {
    ColumnsView::new(
        |app: &App| ColumnsParts {
            sources: &app.columns.listings,
            state: &app.columns.state,
            look: &app.look,
        },
        |app: &mut App| ColumnsPartsMut {
            sources: &app.columns.listings,
            state: &mut app.columns.state,
            look: &app.look,
        },
    )
    .labels(app.columns.labels.0.clone())
    .dnd(crate::dnd::binder(app))
    .drop_action(intents::drop_action)
    // "Show sizes in Columns view", read at each build: the popover's row changes it live.
    .show_sizes(|app: &App| shell::preferences(app).columns_show_sizes)
    .escape_closes(|app: &App| {
        !shell::preferences(app).tenxer_mode && !app.browser.is_chooser_mode()
    })
    .close_buttons(|app: &App| !shell::preferences(app).tenxer_mode)
    .collection(column_collection)
    .pointer_settings(|app: &App| {
        let chooser = app.browser.is_chooser_mode();
        let modes =
            crate::clicks::for_chooser(crate::clicks::columns(shell::preferences(app)), chooser);
        PointerSettings::default()
            .modes(modes)
            .rename(!chooser)
            .multiple(app.chooser.allows_multiple())
    })
    // GTK's column header actions, in its order (`pane_header::actions`).
    .header_actions(|app: &App, cx| pane_header::actions::column(app, &cx))
    // The filter's field, in a band under the header of the column it is open in.
    .header_band(|app: &App, cx| filter::column_band(app, cx.depth))
    // The filter's Enter target among its results, plus copied, cut, and dragging marks.
    .marks(|app: &App, depth, id| {
        crate::dnd::with_marks(
            app,
            id,
            crate::clipboard::with_marks(app, id, filter::marks(app, depth, id)),
        )
    })
    .on_intent(intents::heard)
}

/// Column `depth`'s collection with the part's extras: its status, thumbnails, fills, drags,
/// the window's zoom wheel, its keys and the rename editor.
fn column_collection(app: &App, depth: usize, collection: Collection<App>) -> Collection<App> {
    let collection = collection
        .empty(move |app: &App| shown_status(app, depth))
        .icon(app.columns.thumbs.icon(&app.look))
        .on_drag_phase(|app: &mut App, d, key, tx| crate::dnd::drag_phase(app, d, key, tx))
        .on_visible(move |app: &mut App, range| {
            // Recorded only: the part syncs the store and asks for fills once a poll.
            app.columns.thumbs.see(depth, range.clone());
            app.columns.fills.see(depth, range);
            false
        })
        .on_wheel(crate::appearance::wheel::heard);
    // A column with no rows is described as GTK's directory surface is then.
    let collection = match status(app, depth) {
        Some((said, _)) => collection.description(said.into_owned()),
        None => collection,
    };
    let keys = super::live_keys(shell::preferences(app), app.chooser.allows_multiple());
    let collection = match keys {
        Some(keys) => collection.settings(keys),
        None => collection,
    };
    // The editor answers in the column its edit names, and in no other.
    let collection = match app.columns.scope(depth) {
        Some(scope) => collection.rename(
            move |app: &App| app.rename.state.renaming_in(&scope),
            rename::editor(app).slot(),
        ),
        None => collection,
    };
    commands::in_view(collection)
}
