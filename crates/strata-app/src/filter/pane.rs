// SPDX-License-Identifier: MIT

//! The filter in the one pane of Icons and List: GTK's `filter_controls` in
//! `src/ui/browser_modes/filter.rs`, the pane's in `icons_controls.rs` and `list_pane.rs`.
//!
//! - **Its toggle** is the last of the pane header's actions, "Filter icons (Ctrl+F)" or
//!   "Filter list (Ctrl+F)", and **its field** a band under the header, over the listing, as
//!   GTK's revealer is (`prepare_listing` in `pane_shell.rs`).
//! - **Its results** take the listing's place in a collection of their own named "Search
//!   results" (`inline_search::wrap` and `build_collection`): a grid of tiles in Icons, a list
//!   of rows in List, each with its path under the folder while "Include subfolders" is on.
//!   The pane's listing keeps its own selection and cursor underneath, which the browser holds
//!   as before the filter, so closing the filter gives them back as they were. The results'
//!   collection takes the view's key, so the view's keyboard, focus and commands are theirs
//!   while they show.

use rune::{El, Tx};
use strata_components::{
    collection::{
        element::{CollectionState, Intent, RowMarks},
        selection::{Listing, Position, Selection},
    },
    contract::{Direction, EntryId, EntrySource},
    filter_bar::filter::status as status_line,
    icons_grid::IconsGridState,
};

use super::place::Where;
use crate::{App, handover, listing::Column, menus, shell};

/// GTK's `SEARCH_RESULTS_LABEL`: the results' collection's name.
pub(crate) const SEARCH_RESULTS: &str = "Search results";

/// The results a pane shows in the place of its listing, while it shows them.
pub(crate) struct PaneHits {
    hits: Option<Column>,
    collection: CollectionState,
    /// The Icons results' grid.
    pub(crate) grid: IconsGridState,
    /// Moves each time results take the listing's place, so two sets never share a version.
    swaps: u64,
    /// Whether the results come from the subfolders too: each then shows its path.
    pub(crate) recursive: bool,
}

/// What a pane shows while it has no results.
struct Nothing;

impl EntrySource for Nothing {
    fn len(&self) -> usize {
        0
    }

    fn id_at(&self, index: usize) -> EntryId {
        unreachable!("no result {index}")
    }

    fn index_of(&self, _: EntryId) -> Option<usize> {
        None
    }

    fn entry(&self, index: usize) -> strata_components::contract::Entry<'_> {
        unreachable!("no result {index}")
    }
}

impl PaneHits {
    pub(crate) fn new() -> Self {
        Self {
            hits: None,
            collection: CollectionState::new(),
            grid: IconsGridState::new(),
            swaps: 0,
            recursive: false,
        }
    }

    /// Whether results show.
    pub(crate) fn shows(&self) -> bool {
        self.hits.is_some()
    }

    /// The results and their selection, while they show.
    pub(crate) fn shown_with_selection(&self) -> Option<(&dyn EntrySource, &Selection)> {
        let hits: &dyn EntrySource = self.hits.as_ref()?;
        Some((hits, &self.collection.selection))
    }

    /// The results as the collection reads them.
    pub(crate) fn source(&self) -> &dyn EntrySource {
        match &self.hits {
            Some(hits) => hits,
            None => &Nothing,
        }
    }

    pub(crate) fn collection(&self) -> &CollectionState {
        &self.collection
    }

    pub(crate) fn parts_mut(&mut self) -> (&dyn EntrySource, &mut CollectionState) {
        let source: &dyn EntrySource = match &self.hits {
            Some(hits) => hits,
            None => &Nothing,
        };
        (source, &mut self.collection)
    }

    pub(crate) fn version(&self) -> u64 {
        let version = self.hits.as_ref().map_or(0, Column::version);
        self.swaps.wrapping_shl(40) ^ version
    }

    /// Shows `hits` in the listing's place in the view keyed `scope`, nothing selected when
    /// `clear`. The first set starts the collection afresh, with the view's controllers.
    pub(crate) fn show(&mut self, place: Where, hits: Column, clear: bool, tx: &mut Tx) {
        let scope = place.view_key();
        let first = self.hits.is_none();
        let hits = self.hits.insert(hits);
        if first {
            self.swaps += 1;
            self.collection = crate::single::results_state(place == Where::List);
            let _ = self
                .collection
                .enter(scope, &*hits, None, Listing::Complete, tx);
        } else {
            if clear {
                let _ = self.collection.selection.clear();
            }
            let _ = self
                .collection
                .sync(scope, &*hits, handover::sync_rule(hits), tx);
        }
        self.collection.note_selection(&*hits);
    }

    /// Gives the listing its place back.
    pub(crate) fn drop_hits(&mut self) {
        if self.hits.take().is_some() {
            self.swaps += 1;
        }
    }

    /// The results taken out, with their selection, for the view a switch shows.
    pub(crate) fn take(&mut self) -> Option<(Column, Position)> {
        let position = self.collection.selection.position(self.hits.as_ref()?);
        let hits = self.hits.take()?;
        self.swaps += 1;
        Some((hits, position))
    }

    /// The selection a switch carried over, on the results now shown.
    pub(crate) fn restore(&mut self, position: &Position) {
        let Some(hits) = &self.hits else {
            return;
        };
        let _ = self.collection.selection.restore(position);
        self.collection.note_selection(hits);
    }

    pub(crate) fn reveal_index(&self, scope: &str, index: usize, tx: &mut Tx) {
        if let Some(hits) = &self.hits {
            self.collection.reveal(scope, hits, index, tx);
        }
    }

    /// The cursor on result `id`, which alone is selected, kept in view.
    pub(crate) fn land(&mut self, scope: &str, id: EntryId, tx: &mut Tx) {
        let Some(hits) = &self.hits else {
            return;
        };
        let Some(index) = hits.index_of(id) else {
            return;
        };
        let _ = self.collection.selection.select_only(hits, index);
        self.collection.note_selection(hits);
        self.collection.reveal(scope, hits, index, tx);
    }
}

/// The pane the filter is open in, while it is open in the view shown.
fn open_place(app: &App) -> Option<Where> {
    let open = app.filter.open_in.as_ref()?;
    let shown = Where::shown(app)?;
    (shown == open.place && shown != Where::Columns && app.filter.state.is_open()).then_some(shown)
}

/// The filter's field, in a band under the pane's header while it is open there.
pub(crate) fn band(app: &App) -> Option<El> {
    open_place(app)?;
    super::bar(app).build(app, &app.look)
}

/// The pane header's filter toggle, last of its actions: on opens the filter with the keyboard
/// in it, off closes it. Its key is `<view>-filter`.
pub(crate) fn toggle(app: &App, view: &str) -> Option<El> {
    let place = Where::shown(app).filter(|place| *place != Where::Columns)?;
    let look = &app.look;
    let built = strata_components::header::action(
        look,
        strata_kit::icons::IconName::Filter,
        &super::name_in(place),
    )
    .on_press_tx(|app: &mut App, tx: &mut Tx| toggled(app, tx))
    .build(&look.kit)
    .checked(open_place(app).is_some());
    Some(crate::buttons::on_return(format!("{view}-filter"), built))
}

fn toggled(app: &mut App, tx: &mut Tx) {
    if open_place(app).is_some() {
        let polled = app.filter.state.close();
        super::route(app, polled, tx);
        return;
    }
    super::open(app, None, tx);
}

/// C18's Enter target among the results.
pub(crate) fn marks(app: &App, id: EntryId) -> RowMarks {
    match &app.filter.open_in {
        Some(open) => super::marks(app, open.depth, id),
        None => RowMarks::default(),
    }
}

/// "Searching…" or "No matching files" over results that have none, centred in the pane as
/// GTK's status is.
pub(crate) fn status(app: &App) -> Option<El> {
    let said = super::status(app)?;
    Some(status_line(&app.look, "filter-status", said))
}

/// What the results' collection asks: a result opens as GTK's do (`SearchClick`), its menu is
/// the window's, and Left at the start leaves for the sidebar as from the listing.
pub(crate) fn heard(app: &mut App, intent: Intent, tx: &mut Tx) {
    let Some(depth) = app.filter.open_in.as_ref().map(|open| open.depth) else {
        return;
    };
    shell::publish(app);
    if !super::hit_intent(app, depth, &intent, tx) {
        match intent {
            Intent::ContextMenu { target, anchor } => {
                let request = menus::Request {
                    depth,
                    entry: target,
                    anchor,
                };
                menus::open(app, request, tx);
            }
            // A slow second click on a result's name.
            Intent::BeginRename { id } => crate::rename::begin_on(app, depth, id, tx),
            Intent::Edge(Direction::Left) => {
                shell::leave_view_at_start(app, tx);
            }
            // Drags and new tabs are not yet here.
            _ => {}
        }
    }
    shell::settle(app, tx);
}

/// Result `id` alone selected, with the cursor: a menu's row on it asks.
pub(crate) fn select_only<V: crate::single::SinglePane>(app: &mut App, id: EntryId, tx: &mut Tx) {
    app.filter.pane.land(V::KEY, id, tx);
}

/// Scrolls result `id` into view.
pub(crate) fn reveal<V: crate::single::SinglePane>(app: &App, id: EntryId, tx: &mut Tx) {
    let hits = &app.filter.pane;
    if let Some(index) = hits.source().index_of(id) {
        hits.reveal_index(V::KEY, index, tx);
    }
}

/// The results' path under the folder, while they come from its subfolders too: what each
/// shows under its name (`relative_result_path`).
pub(crate) fn path_of(app: &App, id: EntryId) -> Option<String> {
    if !app.filter.pane.recursive {
        return None;
    }
    let open = app.filter.open_in.as_ref()?;
    let root = open.folder.native_path()?;
    let entry = app.filter.found.get(&id)?;
    let path = entry.location.native_path()?;
    Some(
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned(),
    )
}
