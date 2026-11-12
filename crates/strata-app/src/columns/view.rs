// SPDX-License-Identifier: MIT

//! The Columns view over the adapter's listings and state, with Strata's words.

use rune::{El, col, text};
use strata_components::{
    collection::pointer::PointerSettings,
    columns::{ColumnsLabels, ColumnsParts, ColumnsPartsMut, ColumnsView},
};
use strata_core::i18n::tr;

use super::intents;
use crate::{App, commands, rename};

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
        Self(labels)
    }
}

/// What column `depth` says in place of rows while it shows none ([`crate::surface`]), and
/// whether it draws it.
fn status(app: &App, depth: usize) -> Option<(&str, bool)> {
    let listing = app.columns.listings.columns.get(depth)?;
    app.columns.surface.status(depth, listing)
}

/// What a column draws in place of rows: the empty page's text or the browser's message, as
/// GTK's column draws them (`show_empty` and `show_error` in `src/ui/browser/presentation.rs`).
fn shown_status(app: &App, depth: usize) -> Option<El> {
    let (message, _) = status(app, depth).filter(|(_, drawn)| *drawn)?;
    let look = &app.look;
    Some(
        col().w_full().px(look.space.lg).py(look.space.lg).child(
            text(message.to_owned())
                .font(look.kit.ty.ui)
                .color(look.kit.c.text_muted),
        ),
    )
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
    .collection(|app, depth, collection| {
        let collection = collection.empty(move |app: &App| shown_status(app, depth));
        // A column with no rows is described as GTK's directory surface is then.
        let collection = match status(app, depth) {
            Some((said, _)) => collection.description(said.to_owned()),
            None => collection,
        };
        let collection = match &app.columns.keys {
            Some(keys) => collection.settings(keys.clone()),
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
    })
    .pointer_settings(|app: &App| PointerSettings::default().modes(app.columns.clicks))
    .on_intent(intents::heard)
}
