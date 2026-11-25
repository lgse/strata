// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use rune::{El, Tx};
use strata_components::{
    collection::{element::RowMarks, selection::Selection},
    contract::{EntryId, ListSource},
    filter_bar::filter::{
        FilterBar, FilterIntent, FilterLabels, FilterState, Polled, Results, Shows,
    },
};
use strata_core::{
    i18n::{count, tr},
    model::{FileEntry, Location},
};
use strata_desktop::keys::{
    KeyPress, Keys as KeyLabels, Map, SHORTCUT_MODIFIERS,
    bindings::{COPY, CUT, ESCAPE, ESCAPE_ANY, FILTER, MOVE_TO_TRASH, PASTE, REDO, UNDO},
    label_in_map,
};
use strata_fs::services::{SEARCH_RESULT_LIMIT, SearchItem, SearchLimits};

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, Outcome, Reads, Situation, Stage},
    listing::{Column, Edit, Load, Row},
    shell,
    slots::{Ids, Later, Listener, Part, Place, Setup},
};

mod blur;
mod descent;
mod hits;
pub(crate) mod pane;
mod place;
mod renamed;
pub(crate) mod session;
mod switch;
pub(crate) mod targets;
pub(crate) mod tenxer;
mod typing;

pub(crate) use hits::HitsUpdate;
use pane::PaneHits;
use place::Where;
use renamed::RenamedResult;
use session::{Batch, Input, Scope, Session};
pub(crate) use tenxer::{
    begin_listing_search, clear_listing_filter, commit_listing_filter, commit_listing_search,
    current_hit_path, dismiss_filter_only, dismiss_search_only, end_tenxer_filters,
    focus_visible_results, footer_counts, listing_filter_query, mark as tenxer_mark, results_shown,
    reveal_search_hit, search_active, set_listing_filter, set_listing_search,
    settled_descent_folder,
};
pub(crate) use typing::reads as is_typing_key;

#[cfg(test)]
mod tests;

const KEY: &str = "filter";

const DEBOUNCE: Duration = Duration::from_millis(40);

/// The view and the column the filter is open in, and that column's folder.
#[derive(Clone, Debug, PartialEq)]
struct OpenIn {
    place: Where,
    depth: usize,
    folder: Location,
}

impl OpenIn {
    /// Shows `update` in the place of column `self.depth`'s rows.
    pub(super) fn show_hits(&self, app: &mut App, mut update: HitsUpdate, tx: &mut Tx) {
        update.hits.set_worn(shell::worn(app));
        match self.place {
            Where::Columns => {
                let paths = hits::paths(&app.filter.found);
                let details = app.filter.pane.recursive;
                app.columns.describe_hits(paths, details);
                app.columns.show_hits(self.depth, update, tx);
            }
            place if place.depth_shown(app, self.depth) => {
                if !app.filter.pane.shows() {
                    app.icons.autoscroll().stop_gone(None);
                    app.list.autoscroll().stop_gone(None);
                }
                app.filter.pane.show(place, update, tx);
            }
            _ => {}
        }
    }
}

pub(crate) struct Filter {
    state: FilterState,
    open_in: Option<OpenIn>,
    session: Session,
    /// Ordered search items currently shown, for pruning absent paths after moves or deletions.
    items: Vec<SearchItem>,
    /// Each result's entry, by its id: what a command acts on ([`targets`]).
    found: HashMap<EntryId, FileEntry>,
    /// A rename of a shown result that the next batch follows to its new path.
    renamed: RenamedResult,
    /// The results in a pane of Icons or List.
    pub(crate) pane: PaneHits,
    /// The field's text after its last edit: what tells a `/` typed into it.
    typed: String,
    /// The query a `/` was typed over, until its results settle and its first match opens.
    descent: Option<String>,
    refocus_field: bool,
    /// The field had the keyboard when the tab was hidden.
    back_to_field: bool,
    /// Focus has entered the filtered column since it opened in Columns (`blur`).
    seen_in_column: bool,
    /// 10xer `s` forced recursive path search (`SearchScope::Paths`).
    forced_recursive: bool,
    /// The `f` query `s` borrowed (`listing_search.rs`).
    saved_filter: Option<String>,
    /// Opened by 10xer's `f` or `s` prompt (cleared on leaving 10xer mode).
    tenxer_owned: bool,
    focus_on_arrival: bool,
    /// Counts the columns the filter opened in, for the bar's key.
    serial: u64,
    edits: u64,
    debounce: Rc<RefCell<Option<glib::SourceId>>>,
    later: Later<App>,
    ids: Ids,
    /// What the bar reads while no column is there to read: nothing.
    nothing: (ListSource, Selection),
}

fn named(message: &str, keyless: &str) -> String {
    let keys = label_in_map(
        &rust_i18n::locale(),
        &KeyLabels::One(FILTER.shown()),
        Map::Default,
    );
    if keys.is_empty() {
        return tr(keyless);
    }
    rust_i18n::t!(message, keys = keys).into_owned()
}

/// The Columns column's name for the toggle and the field.
fn column_name() -> String {
    named("Filter this pane (%{keys})", "Filter this pane")
}

/// The toggle's and the field's name in `place`: `filter_controls`' tooltip.
fn name_in(place: Where) -> String {
    match place {
        Where::Columns => column_name(),
        Where::Icons => named("Filter icons (%{keys})", "Filter icons"),
        Where::List => named("Filter list (%{keys})", "Filter list"),
    }
}

/// `filter_placeholder`: the folder's item count.
fn placeholder(items: usize) -> String {
    rust_i18n::t!("Filter %{items}…", items = count("items", items)).into_owned()
}

/// What the bar's Enter and Down read: the open column's results or rows, and its selection.
fn results(app: &App) -> Results<'_> {
    let shown = app
        .filter
        .open_in
        .as_ref()
        .and_then(|open| open.place.shown_with_selection(app, open.depth));
    let (source, selection) = shown.unwrap_or((&app.filter.nothing.0, &app.filter.nothing.1));
    Results { source, selection }
}

fn bar(app: &App) -> FilterBar<App> {
    let open = app.filter.open_in.as_ref();
    let items = open.map_or(0, |open| open.place.listing_len(app, open.depth));
    let place = open.map_or(Where::Columns, |open| open.place);
    let key = format!("{KEY}-{}", app.filter.serial);
    FilterBar::new(
        key,
        |app: &App| &app.filter.state,
        |app: &mut App| &mut app.filter.state,
    )
    .labels(FilterLabels {
        field: name_in(place),
        placeholder: placeholder(items),
    })
    .results(results)
    .previews(|app: &App| targets::preview_result(app).is_some())
    .tab_to_results(false)
    .on_intent(heard)
}

/// Sends `polled`'s intents to the part.
fn route(app: &mut App, polled: Polled, tx: &mut Tx) {
    bar(app).route(app, polled, tx);
}

/// Where `open` targets the filter now, when a filterable view and folder are shown.
fn opening_in(app: &App) -> Option<OpenIn> {
    let place = Where::shown(app)?;
    let depth = place.targeted(app)?;
    let folder = app.browser.location_at(depth)?;
    Some(OpenIn {
        place,
        depth,
        folder,
    })
}

/// Closes the filter in any other column or view and configures [`FilterState`] for `opening`.
fn prepare_open(app: &mut App, opening: OpenIn, tx: &mut Tx) {
    let elsewhere = app
        .filter
        .open_in
        .as_ref()
        .is_some_and(|open| open.depth != opening.depth || open.place != opening.place);
    if elsewhere {
        let polled = app.filter.state.close();
        route(app, polled, tx);
    }
    let shows = match opening.folder.native_path() {
        Some(_) => Shows::Replace,
        None => Shows::InPlace,
    };
    app.filter.state.set_shows(shows);
    app.filter
        .state
        .set_pick_last(opening.place == Where::Columns);
    if app.filter.open_in.as_ref() != Some(&opening) {
        app.filter.serial += 1;
        app.filter.seen_in_column = false;
    }
    if opening.place == Where::Columns {
        app.columns
            .activate_for_filter(opening.depth, &app.look, tx);
        app.columns.hold_header(Some(opening.depth));
    }
    app.filter.open_in = Some(opening);
}

fn open(app: &mut App, query: Option<&str>, tx: &mut Tx) -> bool {
    let Some(opening) = opening_in(app) else {
        return false;
    };
    app.filter.forced_recursive = false;
    app.filter.saved_filter = None;
    app.filter.tenxer_owned = false;
    let (place, depth) = (opening.place, opening.depth);
    prepare_open(app, opening, tx);
    let polled = match query {
        Some(query) => {
            let before = place.listing_position(app, depth);
            app.filter.state.open_with(query, before)
        }
        None => app.filter.state.open(),
    };
    tx.focus_when_shown(bar(app).field_key(), false);
    app.filter.typed = app.filter.state.text().to_owned();
    route(app, polled, tx);
    true
}

/// What the bar asks.
fn heard(app: &mut App, intent: FilterIntent, tx: &mut Tx) {
    match intent {
        FilterIntent::Query { text } => queried(app, text, tx),
        FilterIntent::ShowListing { restore } => {
            stop(app);
            app.filter.typed.clear();
            if let Some(open) = app.filter.open_in.clone() {
                open.place.drop_hits(app, restore.as_ref(), tx);
            }
        }
        FilterIntent::Closed => {
            refocus_on_close(app, tx);
            closed(app);
            crate::preview::follow_filter_close(app);
        }
        FilterIntent::Open { id } => {
            if let Some(open) = app.filter.open_in.clone() {
                shell::publish(app);
                open_hit(app, open.depth, id);
                shell::settle(app, tx);
            }
        }
        FilterIntent::FocusResults { id } => {
            if let Some(open) = app.filter.open_in.clone() {
                open.place.land(app, open.depth, id, tx);
            }
        }
        FilterIntent::Preview => preview_from_field(app, tx),
        _ => {}
    }
}

fn preview_from_field(app: &mut App, tx: &mut Tx) {
    let Some((depth, entry)) = targets::preview_result(app) else {
        return;
    };
    if entry.is_directory() {
        app.browser.navigate(entry.location);
        shell::settle(app, tx);
    } else {
        crate::preview::toggle_entry(app, Some(entry), Some(depth), tx);
    }
}

fn refocus_on_close(app: &App, tx: &mut Tx) {
    let Some(place) = app.filter.open_in.as_ref().map(|open| open.place) else {
        return;
    };
    let focused = tx.focused();
    if focused == Some(bar(app).field_key().as_str()) {
        tx.focus_when_shown(place.view_key(), false);
    } else if place != Where::Columns && focused == Some(place.view_key()) {
        tx.focus_when_shown(place.view_key(), true);
    }
}

fn queried(app: &mut App, text: String, tx: &mut Tx) {
    let before = std::mem::replace(&mut app.filter.typed, text.clone());
    if !app.filter.state.tenxer()
        && let Some(query) = descent::slash_over(&before, &text).map(str::to_owned)
    {
        // The `/` goes no further than the key: the field keeps the query it was typed over.
        app.filter.state.edit.replace_text(before.clone());
        app.filter.state.edit.set_caret(before.len());
        app.filter.typed = before;
        app.filter.descent = Some(query);
        descent::finish(app, tx);
        return;
    }
    app.filter.descent = None;
    searching(app, tx);
    schedule(app, text);
}

fn searching(app: &mut App, tx: &mut Tx) {
    let Some(open) = app.filter.open_in.clone() else {
        return;
    };
    if open.place.shows_hits(app, open.depth) || open.folder.native_path().is_none() {
        return;
    }
    let mut hits = Column::new(open.folder.clone(), true, None);
    hits.set_load(Load::Listed);
    open.show_hits(app, HitsUpdate::plain(hits, true), tx);
}

fn schedule(app: &mut App, text: String) {
    if let Some(earlier) = app.filter.debounce.borrow_mut().take() {
        earlier.remove();
    }
    app.filter.session.expect(&text);
    app.filter.edits += 1;
    let edit = app.filter.edits;
    let later = app.filter.later.clone();
    let waiting = app.filter.debounce.clone();
    let source = glib::timeout_add_local_once(DEBOUNCE, move || {
        waiting.borrow_mut().take();
        later.then(move |app: &mut App, _: &mut Tx| {
            // Queued before a later edit, or before the filter stopped: not this text's turn.
            if app.filter.edits == edit {
                search(app, &text);
            }
            false
        });
    });
    app.filter.debounce.replace(Some(source));
}

/// Runs `text` at once without waiting for `DEBOUNCE` (`FilterQueryBinding::flush`).
fn flush_search(app: &mut App, text: &str) {
    if let Some(earlier) = app.filter.debounce.borrow_mut().take() {
        earlier.remove();
    }
    app.filter.edits += 1;
    app.filter.session.expect(text);
    search(app, text);
}

fn search_scope(
    root: std::path::PathBuf,
    preferences: &strata_desktop::settings::Preferences,
    forced_recursive: bool,
) -> (Scope, bool) {
    if forced_recursive {
        (
            Scope::Paths {
                root,
                recursive: true,
            },
            true,
        )
    } else if preferences.tenxer_mode {
        (
            Scope::Paths {
                root,
                recursive: false,
            },
            false,
        )
    } else {
        let recursive = preferences.filter_include_subfolders;
        (Scope::Folder { root, recursive }, recursive)
    }
}

/// Runs the query `text` over the open column's folder, when it is a local one.
fn search(app: &mut App, text: &str) {
    let (Some(open), true) = (app.filter.open_in.clone(), app.filter.state.is_open()) else {
        return;
    };
    let Some(root) = open.folder.native_path().map(std::path::Path::to_path_buf) else {
        return;
    };
    let preferences = shell::preferences(app);
    let show_hidden = app
        .browser
        .column_preferences(open.depth)
        .unwrap_or_else(|| app.browser.preferences())
        .show_hidden;
    let (scope, recursive) = search_scope(root, preferences, app.filter.forced_recursive);
    let input = Input { scope, show_hidden };
    let limits = limits(preferences);
    app.filter.pane.recursive = recursive;
    let later = app.filter.later.clone();
    app.filter
        .session
        .update(input, text, limits, move |batch| {
            later.then(move |app: &mut App, tx: &mut Tx| delivered(app, batch, tx));
        });
}

fn hidden_toggled(app: &mut App, _: &mut Tx) -> bool {
    refresh_source_filter(app);
    app.filter.state.is_open()
}

pub(crate) fn refresh_source_filter(app: &mut App) {
    if app.filter.state.is_open() {
        let text = app.filter.state.text().trim().to_owned();
        search(app, &text);
    }
}

pub(crate) fn limits(preferences: &strata_desktop::settings::Preferences) -> SearchLimits {
    SearchLimits {
        time_budget: (preferences.search_time_limit != 0)
            .then(|| Duration::from_secs(u64::from(preferences.search_time_limit))),
        max_entries: (preferences.search_entry_limit != 0)
            .then_some(preferences.search_entry_limit as usize),
    }
}

/// The local path of filter result `id`, for thumbnails of hits not in an open browser column.
pub(crate) fn native_path(app: &App, id: EntryId) -> Option<PathBuf> {
    app.filter
        .found
        .get(&id)?
        .location
        .native_path()
        .map(std::path::Path::to_path_buf)
}

fn build_hits(
    listing: Option<&Column>,
    ids: &Ids,
    items: &[SearchItem],
) -> (Vec<Row>, HashMap<EntryId, FileEntry>) {
    let mut rows = Vec::with_capacity(items.len());
    let mut found = HashMap::with_capacity(items.len());
    for item in items {
        let entry = hits::entry_of(item);
        let id = ids.of(&entry.location);
        let mut row = Row::of_with_id(&entry, id);
        if let Some(existing) = listing.and_then(|column| column.row_of(id)) {
            row.keep_key_of(existing);
        }
        rows.push(row);
        found.insert(id, entry);
    }
    (rows, found)
}

/// Builds the results `Column`, updates `found`, and settles any pending rename for `batch`.
fn hits_for_batch(
    filter: &mut Filter,
    folder: Location,
    items: &[SearchItem],
    (rows, found): (Vec<Row>, HashMap<EntryId, FileEntry>),
    batch: &Batch,
) -> HitsUpdate {
    filter.items = items.to_vec();
    filter.found = found;
    let mut hits = Column::new(folder, true, None);
    hits.apply(Edit::Replace(rows));
    hits.set_load(Load::Listed);
    let clear = filter.state.shown(&batch.query);
    let settled = filter.renamed.settle(items, !batch.indexing);
    let relocated = settled
        .filter(|_| !clear)
        .map(|renamed| renamed.ids(&filter.ids));
    HitsUpdate {
        hits,
        clear,
        relocated,
    }
}

fn delivered(app: &mut App, batch: Batch, tx: &mut Tx) -> bool {
    let (Some(open), true) = (app.filter.open_in.clone(), app.filter.state.is_open()) else {
        return false;
    };
    let current = app.filter.state.text().trim() == batch.query;
    if batch.generation != app.filter.session.generation() || !current {
        return false;
    }
    let items = eligible(app, &batch);
    let built = build_hits(open.place.listing(app, open.depth), &app.filter.ids, &items);
    let update = hits_for_batch(&mut app.filter, open.folder.clone(), &items, built, &batch);
    open.show_hits(app, update, tx);
    tenxer::results_arrived(app, tx);
    if let Some(index) = take_reveal(app) {
        open.place.reveal_index(app, open.depth, index, tx);
    }
    if !batch.incomplete {
        if app.filter.descent.as_deref() == Some(batch.query.as_str()) {
            descent::finish(app, tx);
        }
        crate::tenxer::finish_filter_descent(app, tx);
    }
    true
}

fn take_reveal(app: &mut App) -> Option<usize> {
    let filter = &mut app.filter;
    let place = filter.open_in.as_ref().map(|open| (open.place, open.depth));
    let shown = match place {
        Some((Where::Columns, depth)) => app.columns.shown_with_selection(depth),
        Some(_) => filter.pane.shown_with_selection(),
        None => None,
    };
    let (source, selection) = shown.unwrap_or((&filter.nothing.0, &filter.nothing.1));
    filter.state.take_reveal(Results { source, selection })
}

pub(crate) use hits::entry_of;

pub(crate) fn eligible_results(
    browser: &strata_fs::app::Browser,
    session: &session::Session,
    batch: &Batch,
) -> (Vec<SearchItem>, bool) {
    let items: Vec<SearchItem> = batch
        .items
        .iter()
        .filter(|item| browser.allows_entry(&hits::entry_of(item)))
        .take(SEARCH_RESULT_LIMIT)
        .cloned()
        .collect();
    // Type/folder predicates run on the interface thread, after ranking but before the display cap.
    // Continue through lower-ranked candidates rather than starving eligible matches.
    let extended = batch.has_more
        && items.len() < SEARCH_RESULT_LIMIT
        && session.more(&batch.query, batch.candidates.saturating_mul(2));
    (items, extended)
}

fn eligible(app: &App, batch: &Batch) -> Vec<SearchItem> {
    eligible_results(&app.browser, &app.filter.session, batch).0
}

/// Stops the search and forgets its results.
fn stop(app: &mut App) {
    if let Some(waiting) = app.filter.debounce.borrow_mut().take() {
        waiting.remove();
    }
    app.filter.edits += 1;
    app.filter.session.cancel();
    app.filter.items.clear();
    app.filter.found.clear();
    app.filter.renamed.clear();
    app.filter.descent = None;
}

fn prune_stale_results(app: &mut App, _: &mut Tx) -> bool {
    if !shows_live_results(&app.filter) {
        return false;
    }
    let filter = &app.filter;
    let paths: Vec<PathBuf> = filter
        .items
        .iter()
        .map(|item| item.path.clone())
        .filter(|path| !filter.renamed.awaits(path))
        .collect();
    let later = app.filter.later.clone();
    app.filter.session.prune_missing(paths, move |gone| {
        later.then(move |app: &mut App, tx: &mut Tx| apply_pruned_results(app, &gone, tx));
    });
    false
}

/// A search is running and its results show in a pane.
fn shows_live_results(filter: &Filter) -> bool {
    filter.session.is_active() && !filter.items.is_empty() && filter.open_in.is_some()
}

fn apply_pruned_results(app: &mut App, gone: &HashSet<PathBuf>, tx: &mut Tx) -> bool {
    let (Some(open), true) = (app.filter.open_in.clone(), app.filter.state.is_open()) else {
        return false;
    };
    if !app
        .filter
        .items
        .iter()
        .any(|item| gone.contains(&item.path))
    {
        return false;
    }
    let show_hidden = app
        .browser
        .column_preferences(open.depth)
        .unwrap_or_else(|| app.browser.preferences())
        .show_hidden;
    let items: Vec<SearchItem> = app
        .filter
        .items
        .iter()
        .filter(|item| {
            !gone.contains(&item.path)
                && (show_hidden || !item.name.starts_with('.'))
                && app.browser.allows_entry(&hits::entry_of(item))
        })
        .cloned()
        .collect();
    let (rows, found) = build_hits(open.place.listing(app, open.depth), &app.filter.ids, &items);
    app.filter.items = items;
    app.filter.found = found;
    let mut hits = Column::new(open.folder.clone(), true, None);
    hits.apply(Edit::Replace(rows));
    hits.set_load(Load::Listed);
    open.show_hits(app, HitsUpdate::plain(hits, false), tx);
    true
}

pub(crate) fn follow_renamed(app: &mut App, from: &Location, to: Option<&Location>) {
    let Some(open) = app.filter.open_in.as_ref() else {
        return;
    };
    if !open.place.shows_hits(app, open.depth) {
        return;
    }
    let from_id = app.filter.ids.of(from);
    if !app.filter.found.contains_key(&from_id) {
        return;
    }
    let Some(from_path) = from.native_path() else {
        return;
    };
    let to_path = to.and_then(Location::native_path);
    if to.is_some() && to_path.is_none() {
        return;
    }
    app.filter.renamed.expect(from_path, to_path);
}

pub(crate) fn forget_renamed(app: &mut App, from: &Location, to: Option<&Location>) {
    let Some(from_path) = from.native_path() else {
        return;
    };
    let to_path = to.and_then(Location::native_path);
    app.filter.renamed.clear_if(from_path, to_path);
}

fn closed(app: &mut App) {
    stop(app);
    app.filter.refocus_field = false;
    app.filter.seen_in_column = false;
    app.filter.forced_recursive = false;
    app.filter.saved_filter = None;
    app.filter.tenxer_owned = false;
    app.filter.focus_on_arrival = false;
    app.filter.open_in = None;
    app.filter.typed.clear();
    app.filter.pane.drop_hits();
    app.columns.hold_header(None);
}

pub(crate) fn status(app: &App) -> Option<String> {
    let open = app.filter.open_in.as_ref()?;
    if open.place.shows_hits(app, open.depth) && results(app).source.is_empty() {
        return Some(match app.filter.session.awaiting() {
            true => tr("Searching…"),
            false => tr("No matching files"),
        });
    }
    None
}

pub(crate) fn status_element(app: &App) -> Option<El> {
    let said = status(app)?;
    let look = &app.look;
    let line = rune::text(said.clone())
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_muted)
        .a11y_hidden();
    Some(
        rune::col()
            .key("filter-status")
            .role("status")
            .label(said)
            .live_region(rune::LiveRegion::Polite)
            .w_full()
            .px(look.space.lg)
            .py(look.space.lg)
            .child(line),
    )
}

#[cfg(feature = "qa")]
pub(crate) fn outlined(app: &App) -> Option<EntryId> {
    app.filter.open_in.as_ref()?;
    app.filter.state.outlined(results(app))
}

pub(crate) fn marks(app: &App, depth: usize, id: EntryId) -> RowMarks {
    let outlined = app
        .filter
        .open_in
        .as_ref()
        .is_some_and(|open| open.depth == depth)
        && app.filter.state.outlined(results(app)) == Some(id);
    let mut marks = RowMarks::default();
    marks.enter_target = outlined;
    marks
}

/// The user picked among the results.
pub(crate) fn picked(app: &mut App) {
    app.filter.state.pick();
    crate::preview::follow_filter_pick(app);
}

pub(crate) fn open_hit(app: &mut App, depth: usize, id: EntryId) {
    let open = app.filter.open_in.as_ref();
    if open.is_none_or(|open| open.depth != depth) {
        return;
    }
    let Some(entry) = app.filter.found.get(&id).cloned() else {
        return;
    };
    if entry.is_directory() {
        app.browser.navigate(entry.location);
    } else {
        app.browser.open_location(entry.location);
    }
}

fn up_returns_to_field(app: &App) -> bool {
    app.filter.state.is_open() && !app.filter.state.tenxer()
}

/// What a result asks of the view, where it is not a listing's: true when the filter took it.
pub(crate) fn hit_intent(
    app: &mut App,
    depth: usize,
    intent: &strata_components::collection::element::Intent,
    tx: &mut Tx,
) -> bool {
    use strata_components::{collection::element::Intent, contract::Direction};
    match intent {
        // Up from the first result: the field takes the keyboard back.
        Intent::Edge(Direction::Up) if up_returns_to_field(app) => {
            app.filter.state.focus_field();
            tx.focus_when_shown(bar(app).field_key(), false);
            true
        }
        Intent::Activate { id, .. } => {
            open_hit(app, depth, *id);
            true
        }
        Intent::Clicked { id } => {
            if let Some(entry) = app.filter.found.get(id).cloned() {
                if entry.is_directory() {
                    open_hit(app, depth, *id);
                } else if shell::preferences(app).single_click_previews
                    && !crate::preview::one_click_files_in_shown_view(app)
                    && crate::preview::entry_supports_quick_preview(&entry)
                {
                    app.browser.request_preview(entry);
                }
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn column_actions(app: &App, depth: usize) -> Vec<El> {
    let look = &app.look;
    let open_here = app
        .filter
        .open_in
        .as_ref()
        .is_some_and(|open| open.depth == depth)
        && app.filter.state.is_open();
    let toggle = strata_components::header::action(
        look,
        strata_kit::icons::IconName::Filter,
        &column_name(),
    )
    .on_press_tx(move |app: &mut App, tx: &mut Tx| toggled(app, depth, tx))
    .build(&look.accent_icon_kit())
    .checked(open_here)
    .key(format!("columns-filter-{depth}"))
    .focusable(false)
    .no_focus_ring();
    vec![toggle]
}

pub(crate) fn column_band(app: &App, depth: usize) -> Option<El> {
    if shell::preferences(app).tenxer_mode {
        return None;
    }
    let open = app.filter.open_in.as_ref()?;
    let here = open.place == Where::Columns && open.depth == depth && app.filter.state.is_open();
    if !here {
        return None;
    }
    bar(app).build(app, &app.look)
}

/// The toggle: on opens the filter in its column with the keyboard in it, off closes it.
fn toggled(app: &mut App, depth: usize, tx: &mut Tx) {
    let open_here = app
        .filter
        .open_in
        .as_ref()
        .is_some_and(|open| open.depth == depth);
    if open_here && app.filter.state.is_open() {
        let polled = app.filter.state.close();
        route(app, polled, tx);
        return;
    }
    app.columns.hold_header(Some(depth));
    open(app, None, tx);
}

pub(crate) fn clear_for_new_item(app: &mut App, depth: usize, tx: &mut Tx) {
    empty_at(app, depth, tx);
}

pub(crate) fn clear_for_reveal(app: &mut App, depth: usize, tx: &mut Tx) {
    empty_at(app, depth, tx);
}

/// Empties the filter's query when it is open in column `depth`.
fn empty_at(app: &mut App, depth: usize, tx: &mut Tx) {
    let here = app
        .filter
        .open_in
        .as_ref()
        .is_some_and(|open| open.depth == depth);
    if !here || app.filter.state.text().is_empty() {
        return;
    }
    app.filter.state.edit.replace_text("");
    let polled = app.filter.state.text_edited(Default::default());
    route(app, polled, tx);
}

pub(crate) fn close_for_regroup(app: &mut App, tx: &mut Tx) {
    let groups = shell::shown_view(app).is_some_and(|view| view.supports_type_grouping);
    if !groups || app.filter.open_in.is_none() {
        return;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
}

fn follow_columns(app: &mut App, tx: &mut Tx) -> bool {
    let Some(open) = app.filter.open_in.clone() else {
        return false;
    };
    if app.browser.location_at(open.depth).as_ref() == Some(&open.folder) {
        return false;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
    true
}

fn refocus(app: &mut App, tx: &mut Tx) -> bool {
    if !app.filter.refocus_field {
        return false;
    }
    let Some(open) = app
        .filter
        .open_in
        .as_ref()
        .filter(|_| app.filter.state.is_open())
    else {
        app.filter.refocus_field = false;
        return false;
    };
    let listed = app.browser.location_at(open.depth).as_ref() == Some(&open.folder)
        && open.place.listed(app, open.depth);
    if !listed {
        return false;
    }
    app.filter.refocus_field = false;
    app.filter.state.focus_field();
    tx.focus_when_shown(bar(app).field_key(), false);
    true
}

/// Ctrl+F: the filter, in the targeted column (`show_filter`).
fn ctrl_f(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    match open(app, None, tx) {
        true => Outcome::Ran,
        false => Outcome::Declined,
    }
}

fn dismiss(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    let polled = app.filter.state.close();
    route(app, polled, tx);
    Outcome::Ran
}

fn on_results(app: &App, at: &Situation) -> bool {
    let Some(open) = app.filter.open_in.as_ref() else {
        return false;
    };
    at.in_view() && open.place.shows_hits(app, open.depth) && !app.filter.state.text().is_empty()
}

fn search_rows_own_key(press: &KeyPress) -> bool {
    !press.is_any(&[COPY, CUT, PASTE, UNDO, REDO])
        && !press.is(MOVE_TO_TRASH
            .in_map(Map::Default)
            .ignoring(SHORTCUT_MODIFIERS))
}

fn leave_result_key(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    Outcome::Left
}

fn typed(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(query) = call.press.and_then(typing::query) else {
        return Outcome::Declined;
    };
    let query = match query {
        typing::Query::Empty => None,
        typing::Query::Character(character) => Some(character.to_string()),
    };
    match open(app, query.as_deref(), tx) {
        true => Outcome::Ran,
        false => Outcome::Declined,
    }
}

fn types_to_search(app: &App, at: &Situation) -> bool {
    use strata_desktop::keys::Active;
    at.in_view() && shell::live_keys(app).type_to_search_active()
}

impl Part for Filter {
    const PLACE: Place<App> = Place::Overlays(|_, _| None);

    fn new(setup: &Setup<'_>) -> Self {
        let mut state = FilterState::new();
        state.set_tenxer(setup.settings.tenxer_mode);
        Self {
            state,
            open_in: None,
            session: Session::default(),
            items: Vec::new(),
            found: HashMap::new(),
            renamed: RenamedResult::default(),
            pane: PaneHits::new(),
            typed: String::new(),
            descent: None,
            refocus_field: false,
            back_to_field: false,
            seen_in_column: false,
            forced_recursive: false,
            saved_filter: None,
            tenxer_owned: false,
            focus_on_arrival: false,
            serial: 0,
            edits: 0,
            debounce: Rc::default(),
            later: setup.later.clone(),
            ids: setup.ids.clone(),
            nothing: (ListSource::new(Vec::new()), Selection::default()),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            use strata_fs::app::BrowserEvent;
            let moves = matches!(
                event,
                BrowserEvent::Reset
                    | BrowserEvent::ColumnsTruncated { .. }
                    | BrowserEvent::ColumnAdded { .. }
                    | BrowserEvent::ColumnsRelocated { .. }
            );
            if moves {
                heard.then(follow_columns);
            }
            let prunes = match event {
                BrowserEvent::TransferFinished { .. }
                | BrowserEvent::DeletionFinished { .. }
                | BrowserEvent::RenameCompleted { .. } => true,
                BrowserEvent::EntriesSpliced { splices, .. } => {
                    splices.iter().any(|splice| splice.removed > 0)
                }
                _ => false,
            };
            if prunes {
                heard.then(prune_stale_results);
            }
            if matches!(event, BrowserEvent::HiddenToggled { .. }) {
                heard.then(hidden_toggled);
            }
        }))
    }

    fn poll(app: &mut App, _clock: f64, tx: &mut Tx) -> bool {
        let tenxer = shell::preferences(app).tenxer_mode;
        app.filter.state.set_tenxer(tenxer);
        let blurred = blur::follow(app, tx);
        let refocused = refocus(app, tx);
        let polled = app.filter.state.tick();
        let changed = polled.changed;
        route(app, polled, tx);
        blurred | changed | refocused
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("filter", ctrl_f)
            .command(FILTER)
            .keys(&[Keys::at(
                Stage::FilterAndLocationCommands,
                &[FILTER.in_map(Map::Default)],
            )
            .when(|app, at| !shell::preferences(app).tenxer_mode && !at.within("preview"))]),
        Action::new("filter-dismiss", dismiss)
            .command(ESCAPE)
            .keys(&[Keys::at(Stage::FilterAndLocationCommands, &[ESCAPE_ANY]).when(on_results)]),
        Action::new("filter-result-vim", tenxer::result_vim_arrow).keys(&[Keys {
            stage: Stage::SearchResultKeys,
            reads: Reads::Press(|press| tenxer::fallback_vim_arrow(press).is_some()),
            when: on_results,
        }]),
        Action::new("filter-result-keys", leave_result_key).keys(&[Keys {
            stage: Stage::SearchResultKeys,
            reads: Reads::Press(search_rows_own_key),
            when: on_results,
        }]),
        Action::new("type-to-search", typed).keys(&[Keys {
            stage: Stage::TextInput,
            reads: Reads::Press(typing::reads),
            when: types_to_search,
        }]),
    ];

    fn customized(app: &mut App) {
        let worn = shell::worn(app).clone();
        app.filter.pane.set_worn(&worn);
    }

    fn switched(app: &mut App, tx: &mut Tx) {
        switch::follow(app, tx);
    }

    fn tab_hidden(app: &mut App, tx: &mut Tx) {
        switch::tab_hidden(app, tx);
    }

    fn tab_shown(app: &mut App, _: &mut Tx) {
        let tenxer = shell::preferences(app).tenxer_mode;
        app.filter.state.set_tenxer(tenxer);
        let worn = shell::worn(app).clone();
        app.filter.pane.set_worn(&worn);
        switch::tab_shown(app);
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.filter.state.field_focused().then_some(Holds::Filter)
    }
}
