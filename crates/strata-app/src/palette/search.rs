// SPDX-License-Identifier: MIT

//! What the palette searches and says: GTK's `start_search`, `begin_query` and
//! `apply_search_event` (`src/ui/search.rs`), and the words of its panel
//! (`src/ui/search/panel.rs`).

use std::path::PathBuf;

use rune::Tx;
use strata_components::search_palette::{
    Answer, Hint, IdleText, PaletteItem, PaletteMode, PaletteQuery,
};
use strata_core::i18n::{count, tr};
use strata_desktop::keys::{
    Keys,
    bindings::{DOWN, OPEN_CONTAINING_FOLDER, UP},
    shown,
};
use strata_fs::services::SearchItem;
use strata_kit::icons::IconName;

use crate::{
    App,
    filter::{
        limits,
        session::{Batch, Input, Scope},
    },
    location::labels::with_keys,
    shell,
};

/// Ctrl+K's mode over `roots` ([`super::roots`]), in the catalogs' words.
pub(super) fn mode(roots: &[PathBuf]) -> PaletteMode {
    let locations: Vec<String> = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect();
    let prompt = tr("Search files and folders…");
    let idle = tr("Type to search Home and mounted local drives");
    let no_results = tr("No matching files or folders");
    PaletteMode {
        // GTK's panel has no name of its own: the field's words name the dialog.
        title: prompt.clone(),
        field_label: prompt.clone(),
        placeholder: prompt,
        description: Some(
            rust_i18n::t!(
                "Search locations:\n%{locations}\nRemote shares are not included.",
                locations = locations.join("\n")
            )
            .into_owned(),
        ),
        list_label: tr("Search results"),
        idle: Some(IdleText {
            first: idle,
            cleared: tr(
                "Type to search Home and mounted local drives\nFuzzy matching · try a name or path fragment",
            ),
        }),
        searching: tr("Searching…"),
        no_results: no_results.clone(),
        no_results_empty: no_results,
        busy: tr("Indexing files…"),
        partial: tr("Partial results"),
        hints: hints(),
        count: items,
        ..PaletteMode::search(&locations)
    }
}

pub(super) fn items(n: usize) -> String {
    count("items", n)
}

/// `search_footer`: the arrows, Enter, and the key for a result's folder.
pub(super) fn hints() -> Vec<Hint> {
    vec![
        Hint::text(with_keys(
            "%{keys}  navigate",
            None,
            Keys::Any(shown![UP.alternate(0), DOWN.alternate(0)]),
        )),
        Hint::icon(IconName::Enter, tr("open")),
        Hint::icon(
            IconName::FolderOpen,
            with_keys(
                "%{keys}  open containing folder",
                None,
                Keys::One(OPEN_CONTAINING_FOLDER.shown()),
            ),
        ),
    ]
}

/// What the index reads: the roots, hidden entries as the browser shows them, the exclusions.
fn input(app: &App) -> Input {
    Input {
        scope: Scope::Trees {
            roots: app.palette.roots.clone(),
            exclusions: shell::preferences(app).search_exclusions.clone(),
        },
        show_hidden: app.browser.preferences().show_hidden,
    }
}

/// The palette asked for `query`: the index starts as it opens, and each edit asks it again.
/// An empty query asks nothing (`begin_query`).
pub(super) fn asked(app: &mut App, query: &PaletteQuery) {
    app.palette.asked = query.generation;
    let input = input(app);
    let limits = limits(shell::preferences(app));
    let later = app.palette.later.clone();
    let deliver = move |batch: Batch| {
        later.then(move |app: &mut App, tx: &mut Tx| delivered(app, batch, tx));
    };
    let text = query.text.trim();
    if text.is_empty() {
        app.palette.session.prepare(input, limits, deliver);
        app.palette.session.expect("");
        return;
    }
    app.palette.session.update(input, text, limits, deliver);
}

/// A batch for the query the field holds: every result so far, done once the index is read.
fn delivered(app: &mut App, batch: Batch, tx: &mut Tx) -> bool {
    let palette = &mut app.palette;
    if !palette.state.is_open() || batch.generation != palette.session.generation() {
        return false;
    }
    palette.found = batch
        .items
        .iter()
        .map(|item| (id(item), item.clone()))
        .collect();
    let items = batch.items.iter().map(item).collect();
    let mut answer = Answer::new(palette.asked, items);
    if !batch.indexing {
        answer = answer.done();
    }
    if batch.coverage.is_partial() {
        answer = answer.partial(batch.coverage.message());
    }
    palette.state.deliver(answer, tx).changed
}

pub(super) fn id(item: &SearchItem) -> String {
    item.path.to_string_lossy().into_owned()
}

/// `result_row`: the name over the full path, which names the row; no characters marked.
pub(super) fn item(found: &SearchItem) -> PaletteItem {
    let icon = match found.is_directory {
        true => IconName::Folder,
        false => IconName::File,
    };
    let icon_key = match found.is_directory {
        true => crate::entry_icon::FOLDER,
        false => crate::entry_icon::icon_for_name(&found.name),
    };
    let path = id(found);
    PaletteItem::new(path.clone(), found.name.clone(), path)
        .icon(icon)
        .icon_key(icon_key)
}
