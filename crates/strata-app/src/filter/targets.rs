// SPDX-License-Identifier: MIT

//! What the window's file commands act on while a filter's results show: GTK's
//! `selected_search_results` and `selected_search_result` (`src/ui/browser/view_selection.rs`),
//! which every command that acts on files asks before the browser's selection
//! (`confirm_delete` and `begin_rename` in `src/ui/browser/view_edit.rs`, `command_targets`
//! and `focused_target` in `file_commands.rs`, `context_entries` in `context_menu.rs`).
//!
//! The browser never holds the results' selection ([`crate::columns`] keeps it out), and its
//! own selection is the one from before the filter: a command that read the browser would act
//! on what the user picked before filtering. So a command asks here first, and only reads the
//! browser when no results show.

use strata_components::contract::EntryId;
use strata_core::model::FileEntry;

use crate::App;

/// The column showing results, while one does.
fn showing(app: &App) -> Option<usize> {
    let open = app.filter.open_in.as_ref()?;
    app.columns.shows_hits(open.depth).then_some(open.depth)
}

/// The results selected, while results show: what Delete, Move to Trash and the commands on
/// the selection act on. `Some` with none selected, so a command acts on nothing rather than
/// on the browser's selection. `None` while no results show.
pub(crate) fn selected(app: &App) -> Option<Vec<FileEntry>> {
    selected_with_depth(app).map(|(selected, _)| selected)
}

/// [`selected`], with the column the results show in.
pub(crate) fn selected_with_depth(app: &App) -> Option<(Vec<FileEntry>, usize)> {
    let depth = showing(app)?;
    let (source, selection) = app.columns.shown_with_selection(depth)?;
    let selected = (0..source.len())
        .filter(|index| selection.is_selected_at(source, *index))
        .filter_map(|index| app.filter.found.get(&source.id_at(index)).cloned())
        .collect();
    Some((selected, depth))
}

/// The result under the results' cursor, and its column: what F2 renames while results show.
/// `None` while no results show, or none has the cursor.
pub(crate) fn under_cursor(app: &App) -> Option<(usize, FileEntry)> {
    let depth = showing(app)?;
    let (_, selection) = app.columns.shown_with_selection(depth)?;
    let entry = app.filter.found.get(&selection.cursor()?)?;
    Some((depth, entry.clone()))
}

/// Result `id`'s entry, when column `depth` shows results.
pub(crate) fn at(app: &App, depth: usize, id: EntryId) -> Option<FileEntry> {
    (showing(app)? == depth)
        .then(|| app.filter.found.get(&id).cloned())
        .flatten()
}

/// What a menu opened on result `id` acts on (`context_entries`): the results selected when
/// it is one of them, else it alone. `None` when column `depth` shows no results.
pub(crate) fn of_row(app: &App, depth: usize, id: EntryId) -> Option<Vec<FileEntry>> {
    let entry = at(app, depth, id)?;
    let selected = selected(app).unwrap_or_default();
    let within = selected
        .iter()
        .any(|other| other.location == entry.location);
    Some(if within { selected } else { vec![entry] })
}
