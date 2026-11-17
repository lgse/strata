// SPDX-License-Identifier: MIT

//! 10xer mode's hooks into [`super::Filter`]: the footer's `filter:` and `search:` marks,
//! driving listing filters (`f`) and subfolder searches (`s`) from C18's `FooterPrompt`, the
//! subfolder search hit path under the cursor, Escape's dismissal of `f` vs `s`, and non-Latin
//! `h j k l` moves on search results (`Stage::SearchResultKeys`).

use rune::Tx;
use strata_components::{
    collection::{
        keyboard::{Command as NavCommand, Target as NavTarget},
        selection::{AnchorRule, Position},
    },
    contract::Direction,
    filter_bar::filter::Polled,
    footer::FilterMark,
};
use strata_core::model::Location;
use strata_desktop::keys::{
    KeyPress,
    bindings::{DOWN, LEFT, RIGHT, UP},
};

use super::{OpenIn, flush_search, place::Where, prepare_open, results, route, targets};
use crate::{
    App,
    commands::{Call, Outcome},
    shell,
};

fn open_shown(app: &App, open: &OpenIn) -> bool {
    Where::shown(app) == Some(open.place)
        && match open.place {
            Where::Columns => crate::tenxer::focused_listing_depth(app) == Some(open.depth),
            _ => open.place.depth_shown(app, open.depth),
        }
}

/// The footer's `filter: …` or `search: …` mark while 10xer mode or a subfolder search is
/// active (`FilterMark`).
pub(crate) fn mark(app: &App) -> Option<FilterMark> {
    let open = app.filter.open_in.as_ref()?;
    if !app.filter.state.is_open() {
        return None;
    }
    let query = app.filter.state.text();
    if query.trim().is_empty() || !open_shown(app, open) {
        return None;
    }
    if app.filter.forced_recursive {
        Some(FilterMark::Search(query.to_owned()))
    } else if shell::preferences(app).tenxer_mode {
        Some(FilterMark::Filter(query.to_owned()))
    } else {
        None
    }
}

/// The subfolder search hit under the cursor, as a path below the searched folder.
pub(crate) fn current_hit_path(app: &App) -> Option<String> {
    if !app.filter.forced_recursive {
        return None;
    }
    let open = app.filter.open_in.as_ref()?;
    if !open_shown(app, open) {
        return None;
    }
    let root = open.folder.native_path()?;
    let (_, entry) = targets::under_cursor(app)?;
    let path = entry.location.native_path()?;
    Some(
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned(),
    )
}

fn non_empty_open(app: &App) -> bool {
    app.filter.state.is_open() && !app.filter.state.text().trim().is_empty()
}

/// Whether a subfolder search (`s`) is open in the shown column.
pub(crate) fn search_active(app: &App) -> bool {
    app.filter.forced_recursive && non_empty_open(app)
}

/// Dismisses an open subfolder search (`s`), restoring any `f` filter it borrowed.
pub(crate) fn dismiss_search_only(app: &mut App, tx: &mut Tx) -> bool {
    let borrowed = app.filter.saved_filter.take();
    if !app.filter.forced_recursive && borrowed.is_none() {
        return false;
    }
    app.filter.forced_recursive = false;
    let saved = borrowed.unwrap_or_default();
    if saved.trim().is_empty() {
        let polled = app.filter.state.close();
        route(app, polled, tx);
    } else {
        apply_tenxer_query(app, &saved, Apply::COMMIT_FILTER, tx);
        focus_results_or_view(app, tx);
    }
    true
}

/// Clears an open listing filter (`f`).
pub(crate) fn dismiss_filter_only(app: &mut App, tx: &mut Tx) -> bool {
    if app.filter.forced_recursive || !non_empty_open(app) {
        return false;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
    true
}

fn opening_in_tenxer(app: &App) -> Option<OpenIn> {
    let place = Where::shown(app)?;
    let depth = match place {
        Where::Columns => crate::tenxer::focused_listing_depth(app)?,
        pane => pane.pane(app)?.depth()?,
    };
    let folder = app.browser.location_at(depth)?;
    Some(OpenIn {
        place,
        depth,
        folder,
    })
}

/// Whether the filter is open on `target`.
fn open_on(app: &App, target: &OpenIn) -> bool {
    app.filter.open_in.as_ref() == Some(target) && app.filter.state.is_open()
}

/// `BrowserView::listing_filter`: the current or saved `f` query in the focused listing.
pub(crate) fn listing_filter_query(app: &App) -> Option<String> {
    let target = opening_in_tenxer(app)?;
    if !open_on(app, &target) {
        return None;
    }
    let text = if app.filter.forced_recursive {
        app.filter.saved_filter.clone().unwrap_or_default()
    } else {
        app.filter.state.text().to_owned()
    };
    (!text.trim().is_empty()).then_some(text)
}

/// How a 10xer prompt's query reaches the filter: `forced` for `s`'s recursive search, and
/// `flush` to run it at once on Enter.
#[derive(Clone, Copy)]
struct Apply {
    forced: bool,
    flush: bool,
}

impl Apply {
    const FILTER: Self = Self {
        forced: false,
        flush: false,
    };
    const COMMIT_FILTER: Self = Self {
        forced: false,
        flush: true,
    };
    const SEARCH: Self = Self {
        forced: true,
        flush: false,
    };
    const COMMIT_SEARCH: Self = Self {
        forced: true,
        flush: true,
    };
}

fn apply_tenxer_query(app: &mut App, query: &str, how: Apply, tx: &mut Tx) {
    let Some(opening) = opening_in_tenxer(app) else {
        return;
    };
    if query.trim().is_empty() && !how.forced {
        if open_on(app, &opening) {
            let polled = app.filter.state.close();
            route(app, polled, tx);
        }
        return;
    }
    let before = opening.place.listing_position(app, opening.depth);
    prepare_open(app, opening, tx);
    app.filter.forced_recursive = how.forced;
    app.filter.tenxer_owned = true;
    let polled = edit_query(app, query, before);
    app.filter.state.edit.focused = false;
    app.filter.typed = app.filter.state.text().to_owned();
    route(app, polled, tx);
    if how.flush && !query.trim().is_empty() {
        flush_search(app, query);
    }
}

/// Opens the filter on `query`, or replaces the open one's text with it, keeping `before` as
/// the selection Escape gives back.
fn edit_query(app: &mut App, query: &str, before: Position) -> Polled {
    let state = &mut app.filter.state;
    if !state.is_open() {
        return state.open_with(query, before);
    }
    state.edit.replace_text(query.to_owned());
    state.edit.set_caret(query.len());
    state.text_edited(before)
}

/// `BrowserView::set_listing_filter`: updates the focused listing's `f` filter live.
pub(crate) fn set_listing_filter(app: &mut App, query: &str, tx: &mut Tx) {
    app.filter.saved_filter = None;
    apply_tenxer_query(app, query, Apply::FILTER, tx);
}

/// `BrowserView::commit_listing_filter`: flushes `query` and returns focus to the results/view.
pub(crate) fn commit_listing_filter(app: &mut App, query: &str, tx: &mut Tx) {
    app.filter.saved_filter = None;
    apply_tenxer_query(app, query, Apply::COMMIT_FILTER, tx);
    focus_results_or_view(app, tx);
}

/// `BrowserView::clear_listing_filter`: clears an active `f` filter and focuses the listing.
pub(crate) fn clear_listing_filter(app: &mut App, tx: &mut Tx) -> bool {
    let Some(target) = opening_in_tenxer(app) else {
        return false;
    };
    if !open_on(app, &target) || app.filter.state.text().is_empty() {
        return false;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
    focus_results_or_view(app, tx);
    true
}

/// `BrowserView::begin_listing_search`: starts `s` on a local folder, saving any active `f` query.
pub(crate) fn begin_listing_search(app: &mut App) -> Option<String> {
    let target = opening_in_tenxer(app)?;
    target.folder.native_path()?;
    if app.filter.open_in.as_ref() == Some(&target) && search_active(app) {
        return Some(app.filter.state.text().to_owned());
    }
    let saved = listing_filter_query(app).unwrap_or_default();
    app.filter.saved_filter = Some(saved);
    Some(String::new())
}

/// `BrowserView::set_listing_search`: updates `s` hits live, or previews the saved `f` filter when empty.
pub(crate) fn set_listing_search(app: &mut App, query: &str, tx: &mut Tx) {
    if query.trim().is_empty() {
        let saved = app.filter.saved_filter.clone().unwrap_or_default();
        apply_tenxer_query(app, &saved, Apply::FILTER, tx);
        return;
    }
    let _ = crate::tenxer::end_all_ranges(app);
    apply_tenxer_query(app, query, Apply::SEARCH, tx);
}

/// `BrowserView::commit_listing_search`: commits `query`, or dismisses `s` when empty.
pub(crate) fn commit_listing_search(app: &mut App, query: &str, tx: &mut Tx) {
    if query.trim().is_empty() {
        if !dismiss_search_only(app, tx) {
            focus_results_or_view(app, tx);
        }
        return;
    }
    apply_tenxer_query(app, query, Apply::COMMIT_SEARCH, tx);
    focus_results_or_view(app, tx);
}

/// Clears filters opened by 10xer's `f` or `s` prompts when leaving 10xer mode, keeping a `Ctrl+F`
/// filter opened before entering 10xer mode (`end_tenxer_session`).
pub(crate) fn end_tenxer_filters(app: &mut App, tx: &mut Tx) {
    app.filter.saved_filter = None;
    app.filter.focus_on_arrival = false;
    if app.filter.tenxer_owned || app.filter.forced_recursive {
        let polled = app.filter.state.close();
        route(app, polled, tx);
    }
}

/// Whether filter or search results show in place of the listing.
pub(crate) fn results_shown(app: &App) -> bool {
    app.filter
        .open_in
        .as_ref()
        .is_some_and(|open| open.place.shows_hits(app, open.depth))
}

/// Focuses visible filter/search results in the shown view, returning `false` when none show.
pub(crate) fn focus_visible_results(app: &App, tx: &mut Tx) -> bool {
    let Some(open) = app.filter.open_in.as_ref() else {
        return false;
    };
    if !open.place.shows_hits(app, open.depth) {
        return false;
    }
    tx.focus_when_shown(open.place.view_key(), true);
    true
}

/// GTK's `focus_filter_results`: with no cursor among the shown results, the keyboard lands on
/// the first one without selecting it, as a GTK list's `grab_focus` does.
fn cursor_on_first_result(app: &mut App, tx: &mut Tx) {
    let Some(open) = app.filter.open_in.clone() else {
        return;
    };
    let without_cursor = open.place.shows_hits(app, open.depth)
        && open
            .place
            .shown_with_selection(app, open.depth)
            .is_some_and(|(source, selection)| {
                !source.is_empty() && selection.cursor_index(source).is_none()
            });
    if !without_cursor {
        return;
    }
    if open.place == Where::Columns {
        app.columns.move_cursor_to(open.depth, 0, tx);
        return;
    }
    {
        let (source, collection) = app.filter.pane.parts_mut();
        let _ = collection
            .selection
            .move_cursor(source, 0, AnchorRule::Keep);
        collection.note_selection(source);
    }
    app.filter.pane.reveal_index(open.place.view_key(), 0, tx);
}

/// GTK's `focus_arrived_results`: results that arrive after a commit, while the keyboard
/// still waits in the view, put the cursor on the first of them.
pub(super) fn results_arrived(app: &mut App, tx: &mut Tx) {
    if !app.filter.focus_on_arrival {
        return;
    }
    let waiting = app
        .filter
        .open_in
        .as_ref()
        .is_some_and(|open| tx.focused().is_none_or(|key| key == open.place.view_key()));
    if waiting {
        cursor_on_first_result(app, tx);
    } else {
        app.filter.focus_on_arrival = false;
    }
}

fn focus_results_or_view(app: &mut App, tx: &mut Tx) {
    app.filter.focus_on_arrival = app.filter.open_in.is_some();
    cursor_on_first_result(app, tx);
    if !focus_visible_results(app, tx)
        && let Some(view) = shell::view_focus(app)
    {
        tx.focus_when_shown(view, true);
    }
}

/// Resolves the settled directory match for a `/` typed into 10xer's `f` prompt: `None` while
/// results are still arriving, `Some(Some(folder))` when the settled first match is a folder, and
/// `Some(None)` when settled on a file or empty results (`settled_filter_match`).
pub(crate) fn settled_descent_folder(app: &App, at_cursor: bool) -> Option<Option<Location>> {
    if app.filter.open_in.is_none() {
        return Some(None);
    }
    if at_cursor && let Some((_, entry)) = targets::under_cursor(app) {
        return Some(entry.is_directory().then_some(entry.location));
    }
    if app.filter.debounce.borrow().is_some() || app.filter.session.awaiting() {
        return None;
    }
    let shown = results(app).source;
    let folder = (!shown.is_empty())
        .then(|| shown.id_at(0))
        .and_then(|id| app.filter.found.get(&id))
        .filter(|entry| entry.is_directory())
        .map(|entry| entry.location.clone());
    Some(folder)
}

/// `reveal_listing_search_hit` (`src/ui/browser/listing_search.rs`): ends an active `s`
/// search and opens the hit's parent folder with the hit selected.
pub(crate) fn reveal_search_hit(app: &mut App, tx: &mut Tx) -> bool {
    if !search_active(app) {
        return false;
    }
    let Some((_, entry)) = targets::under_cursor(app) else {
        return false;
    };
    let Some(folder) = entry.location.parent() else {
        return false;
    };
    app.filter.saved_filter = None;
    let polled = app.filter.state.close();
    route(app, polled, tx);
    shell::publish(app);
    app.browser.reveal_locations(folder, vec![entry.location]);
    if let Some(view) = shell::shown_view(app)
        && (view.shows)("columns")
    {
        app.columns
            .follow_active(app.browser.active_depth(), &app.look, tx);
    }
    true
}

/// Non-Latin fallback `h j k l` on search results (`search_result_keys`'s `vim_arrow` where
/// C1c lacks the offscreen key's physical origin).
pub(super) fn fallback_vim_arrow(press: &KeyPress) -> Option<Direction> {
    if !press.used_fallback() {
        return None;
    }
    if press.is(DOWN.alternates()) {
        Some(Direction::Down)
    } else if press.is(UP.alternates()) {
        Some(Direction::Up)
    } else if press.is(LEFT.alternates()) {
        Some(Direction::Left)
    } else if press.is(RIGHT.alternates()) {
        Some(Direction::Right)
    } else {
        None
    }
}

pub(super) fn result_vim_arrow(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let Some(dir) = call.press.and_then(fallback_vim_arrow) else {
        return Outcome::Declined;
    };
    let prefs = shell::preferences(app);
    if !prefs.tenxer_mode && prefs.type_to_search {
        return Outcome::Declined;
    }
    let target = NavTarget::Step(dir);
    let command = if prefs.tenxer_mode {
        NavCommand::Cursor(target)
    } else {
        NavCommand::Select(target)
    };
    if crate::tenxer::run_collection_command(app, command, tx) {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}
