// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_components::search_palette::{Answer, PaletteMode, PaletteQuery, ResultKind};
use strata_core::i18n::tr;

use super::search;
use crate::App;

pub(super) fn mode() -> PaletteMode {
    let prompt = tr("Jump to a folder…");
    let searching = tr("Searching…");
    PaletteMode {
        title: prompt.clone(),
        field_label: prompt.clone(),
        placeholder: prompt,
        description: Some(tr("Folders previously visited in Strata")),
        list_label: tr("Search results"),
        searching: searching.clone(),
        no_results: tr("No matching folders"),
        no_results_empty: tr("No folder history yet"),
        busy: searching,
        partial: tr("Partial results"),
        hints: search::hints(),
        count: search::items,
        ..PaletteMode::jump()
    }
}

/// Whether the palette is open on the folder jump.
pub(super) fn is_open(app: &App) -> bool {
    app.palette
        .state
        .mode()
        .is_some_and(|mode| mode.kind == ResultKind::Folders)
}

pub(super) fn asked(app: &mut App, query: &PaletteQuery, tx: &mut Tx) {
    let palette = &mut app.palette;
    let found = palette.history.search(&query.text);
    palette.found = found
        .iter()
        .map(|item| (search::id(item), item.clone()))
        .collect();
    let items = found.iter().map(search::item).collect();
    let answer = Answer::new(query.generation, items).done();
    let _ = palette.state.deliver(answer, tx);
}
