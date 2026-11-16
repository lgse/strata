// SPDX-License-Identifier: MIT

//! 10xer mode's hooks into [`super::Filter`]: the footer's `filter:` and `search:` marks, the
//! subfolder search hit path under the cursor, Escape's dismissal of `f` vs `s`, and non-Latin
//! `h j k l` moves on search results (`Stage::SearchResultKeys`).

use rune::Tx;
use strata_components::{
    collection::keyboard::{Command as NavCommand, Target as NavTarget},
    contract::Direction,
    footer::FilterMark,
};
use strata_desktop::keys::{
    KeyPress,
    bindings::{DOWN, LEFT, RIGHT, UP},
};

use super::{route, targets};
use crate::{
    App,
    commands::{Call, Outcome},
    shell,
};

/// The footer's `filter: …` or `search: …` mark while 10xer mode or a subfolder search is
/// active (`FilterMark`).
pub(crate) fn mark(app: &App) -> Option<FilterMark> {
    let open = app.filter.open_in.as_ref()?;
    if !app.filter.state.is_open() {
        return None;
    }
    let query = app.filter.state.text();
    if query.is_empty() || !open.place.depth_shown(app, open.depth) {
        return None;
    }
    if app.filter.pane.recursive {
        Some(FilterMark::Search(query.to_owned()))
    } else if shell::preferences(app).tenxer_mode {
        Some(FilterMark::Filter(query.to_owned()))
    } else {
        None
    }
}

/// The subfolder search hit under the cursor, as a path below the searched folder.
pub(crate) fn current_hit_path(app: &App) -> Option<String> {
    if !app.filter.pane.recursive {
        return None;
    }
    let open = app.filter.open_in.as_ref()?;
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
    app.filter.state.is_open() && !app.filter.state.text().is_empty()
}

/// Whether a subfolder search (`s`) is open in the shown column.
pub(crate) fn search_active(app: &App) -> bool {
    app.filter.pane.recursive && non_empty_open(app)
}

/// Dismisses an open subfolder search (`s`).
pub(crate) fn dismiss_search_only(app: &mut App, tx: &mut Tx) -> bool {
    if !search_active(app) {
        return false;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
    true
}

/// Clears an open listing filter (`f`).
pub(crate) fn dismiss_filter_only(app: &mut App, tx: &mut Tx) -> bool {
    if app.filter.pane.recursive || !non_empty_open(app) {
        return false;
    }
    let polled = app.filter.state.close();
    route(app, polled, tx);
    true
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
