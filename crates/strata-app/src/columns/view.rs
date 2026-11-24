// SPDX-License-Identifier: MIT

//! The Columns view over the adapter's listings and state, with Strata's words.

use std::borrow::Cow;

use rune::El;
use strata_components::{
    collection::{
        element::{Collection, RowMarks},
        pointer::PointerSettings,
    },
    columns::{ColumnsLabels, ColumnsParts, ColumnsPartsMut, ColumnsView},
    contract::EntryId,
};
use strata_core::i18n::tr;

use super::intents;
use crate::{App, commands, filter, pane_header, rename, shell, surface, tenxer::highlight};

/// The view's words, translated once: the language changes only on restart.
pub(super) struct Labels(ColumnsLabels);

impl Labels {
    pub(super) fn translated() -> Self {
        let mut labels = ColumnsLabels::default();
        labels.strip = tr("Columns");
        labels.close = tr("Close this pane");
        labels.loading = tr("Loading");
        labels.width = tr("Column width");
        labels.pane = tr("Columns view");
        labels.list = tr("Files");
        labels.reveal = rust_i18n::t!("Reveal %{value1} column", value1 = "{name}").into_owned();
        Self(labels)
    }
}

fn status(app: &App, depth: usize) -> Option<(Cow<'_, str>, bool)> {
    if app.columns.listings.hits_at(depth) {
        return filter::status(app).map(|said| (Cow::Owned(said), true));
    }
    let listing = app.columns.listings.columns.get(depth)?;
    app.columns.surface.status(depth, listing)
}

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
    .content_types(crate::dnd::CONTENT_TYPES)
    .drop_action(intents::drop_action)
    // "Show sizes in Columns view", read at each build: the popover's row changes it live.
    .show_sizes(|app: &App| shell::preferences(app).columns_show_sizes)
    .trailing_edge(|app: &App| !crate::preview::draws_divider(app))
    .active_header(|app: &App| !crate::preview::owns_focus(app))
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
            .drags(!chooser)
            .multiple(app.chooser.allows_multiple())
    })
    .header_actions(|app: &App, cx| pane_header::actions::column(app, &cx))
    // The filter's field, in a band under the header of the column it is open in.
    .header_band(|app: &App, cx| filter::column_band(app, cx.depth))
    .marks(row_marks)
    .extra_description(crate::file_providers::badge_version, |app: &App, id| {
        crate::file_providers::badge_description(app, id).map(str::to_owned)
    })
    .on_intent(intents::heard)
}

pub(crate) fn row_marks(app: &App, depth: usize, id: EntryId) -> RowMarks {
    let marks = filter::marks(app, depth, id);
    let marks = if app.columns.shows_hits(depth) {
        highlight::hit_marks(app, id, marks)
    } else {
        let row = app
            .columns
            .listing(depth)
            .and_then(|column| column.row_of(id));
        highlight::listing_marks(app, row.map(crate::listing::Row::name), marks)
    };
    crate::dnd::with_marks(app, id, crate::clipboard::with_marks(app, id, marks))
}

fn column_collection(app: &App, depth: usize, collection: Collection<App>) -> Collection<App> {
    let collection = collection
        .empty(move |app: &App| shown_status(app, depth))
        .icon(app.columns.thumbs.icon(&app.look, depth))
        .on_drag_phase(|app: &mut App, d, key, tx| crate::dnd::drag_phase(app, d, key, tx))
        .on_visible(move |app: &mut App, range| {
            // Recorded only: the part syncs the store and asks for fills once a poll.
            app.columns.thumbs.see(depth, range.clone());
            app.columns.fills.see(depth, range);
            false
        });
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
