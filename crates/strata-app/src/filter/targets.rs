// SPDX-License-Identifier: MIT

use strata_components::{
    collection::selection::Selection,
    contract::{EntryId, EntrySource},
};
use strata_core::model::FileEntry;

use crate::App;

/// The column showing results, while one does.
fn showing(app: &App) -> Option<usize> {
    let open = app.filter.open_in.as_ref()?;
    open.place.shows_hits(app, open.depth).then_some(open.depth)
}

/// What the results' column shows, and their selection.
fn shown(app: &App, depth: usize) -> Option<(&dyn EntrySource, &Selection)> {
    let open = app.filter.open_in.as_ref()?;
    open.place.shown_with_selection(app, depth)
}

pub(crate) fn selected(app: &App) -> Option<Vec<FileEntry>> {
    selected_with_depth(app).map(|(selected, _)| selected)
}

/// [`selected`], with the column the results show in.
pub(crate) fn selected_with_depth(app: &App) -> Option<(Vec<FileEntry>, usize)> {
    let depth = showing(app)?;
    let (source, selection) = shown(app, depth)?;
    let selected = (0..source.len())
        .filter(|index| selection.is_selected_at(source, *index))
        .filter_map(|index| app.filter.found.get(&source.id_at(index)).cloned())
        .collect();
    Some((selected, depth))
}

pub(crate) fn under_cursor(app: &App) -> Option<(usize, FileEntry)> {
    let depth = showing(app)?;
    let (_, selection) = shown(app, depth)?;
    let entry = app.filter.found.get(&selection.cursor()?)?;
    Some((depth, entry.clone()))
}

pub(crate) fn preview_result(app: &App) -> Option<(usize, FileEntry)> {
    let depth = showing(app)?;
    let (source, selection) = shown(app, depth)?;
    let id = selection
        .cursor()
        .filter(|id| selection.contains(source, *id))
        .or_else(|| {
            (0..source.len())
                .find(|index| selection.is_selected_at(source, *index))
                .map(|index| source.id_at(index))
        })?;
    let entry = app.filter.found.get(&id)?.clone();
    Some((depth, entry))
}

/// Result `id`'s entry, when column `depth` shows results.
pub(crate) fn at(app: &App, depth: usize, id: EntryId) -> Option<FileEntry> {
    (showing(app)? == depth)
        .then(|| app.filter.found.get(&id).cloned())
        .flatten()
}

pub(crate) fn of_row(app: &App, depth: usize, id: EntryId) -> Option<Vec<FileEntry>> {
    let entry = at(app, depth, id)?;
    let selected = selected(app).unwrap_or_default();
    let within = selected
        .iter()
        .any(|other| other.location == entry.location);
    Some(if within { selected } else { vec![entry] })
}
