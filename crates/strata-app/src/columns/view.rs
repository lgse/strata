// SPDX-License-Identifier: MIT

//! The Columns view over the adapter's listings and state, with Strata's words.

use rune::{El, col, text};
use strata_components::{
    collection::pointer::PointerSettings,
    columns::{ColumnsLabels, ColumnsParts, ColumnsPartsMut, ColumnsView},
};
use strata_core::i18n::tr;

use super::{intents, listing::Load};
use crate::{App, rename};

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

/// What a column shows in place of rows when its folder could not be read: the browser's
/// message.
fn failure(app: &App, depth: usize) -> Option<El> {
    let Load::Failed(message) = app.columns.listings.columns.get(depth)?.load() else {
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
        let collection = collection.empty(move |app: &App| failure(app, depth));
        let collection = match &app.columns.keys {
            Some(keys) => collection.settings(keys.clone()),
            None => collection,
        };
        // The editor answers in the column its edit names, and in no other.
        match app.columns.scope(depth) {
            Some(scope) => collection.rename(
                move |app: &App| app.rename.state.renaming_in(&scope),
                rename::editor(app).slot(),
            ),
            None => collection,
        }
    })
    .pointer_settings(|app: &App| PointerSettings::default().modes(app.columns.clicks))
    .on_intent(intents::heard)
}
