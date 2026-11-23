// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::model::Location;
use strata_desktop::keys::{
    KeyPress, Map, Modifiers, SHORTCUT_MODIFIERS,
    bindings::{
        BACKSPACE, ENTER, HIDDEN_FILES, HISTORY_BACK, HISTORY_FORWARD, HOME_FOLDER, PARENT_FOLDER,
        REFRESH, SELECT_ALL,
    },
};

use super::{Action, Call, Keys, On, Outcome, Reads, Situation, Stage};
use crate::{
    App,
    appearance::{text_size, views},
    shell,
};

const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
const SUPER: Modifiers = Modifiers::SUPER_MASK;

/// The column a call is for: the row's, the folder's, or the browser's active one.
fn depth(app: &App, on: &On) -> Option<usize> {
    match *on {
        On::Row((depth, _)) | On::Column(depth) => Some(depth),
        On::Focus => app.browser.active_depth(),
        On::Place(_) => None,
    }
}

fn toggle_hidden(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.toggle_hidden();
    Outcome::Ran
}

/// F5 reads every column again (`refresh_browser`); the pane menu's row reads its own.
fn refresh(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let column = match call.on {
        On::Row((depth, _)) | On::Column(depth) => Some(depth),
        On::Focus => None,
        On::Place(_) => return Outcome::Declined,
    };
    strata_fs::adapters::trash_support::forget_all();
    match column {
        Some(depth) => app.browser.retry_column(depth),
        None => app.browser.refresh_all(),
    }
    Outcome::Ran
}

fn open(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let On::Row(row) = call.on else {
        return Outcome::Declined;
    };
    if crate::open_with::open_selection_default(app, row.0, row.1) {
        return Outcome::Ran;
    }
    let Some(view) = shell::shown_view(app) else {
        return Outcome::Declined;
    };
    (view.rows.activate)(app, row);
    Outcome::Ran
}

/// The pane menu's Select All. Ctrl+A in a view is the view's own key.
fn select_all(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let (Some(depth), Some(view)) = (depth(app, &call.on), shell::shown_view(app)) else {
        return Outcome::Declined;
    };
    (view.rows.select_all)(app, depth);
    Outcome::Ran
}

fn back(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.back();
    Outcome::Ran
}

fn forward(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.forward();
    Outcome::Ran
}

fn parent(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.parent();
    Outcome::Ran
}

fn up(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    let columns = shell::shown_view(app).is_some_and(|view| (view.shows)("columns"));
    let browser = app.browser.clone();
    if !columns {
        browser.parent();
        return Outcome::Ran;
    }
    match browser.active_depth() {
        Some(0) => {
            if let Some(parent) = browser.location_at(0).and_then(|first| first.parent()) {
                browser.navigate_to_ancestor(parent);
            }
        }
        Some(depth) => browser.close_column(depth),
        None => {}
    }
    Outcome::Ran
}

fn backspace(press: &KeyPress) -> bool {
    press.is(BACKSPACE.ignoring(SHIFT.union(SUPER)))
}

fn from_the_listing(_: &App, at: &Situation) -> bool {
    at.in_view()
}

fn home(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser
        .navigate(Location::local(crate::launch::home_directory()));
    Outcome::Ran
}

/// The ids are the menu rows' where a row runs the command (`menus::ROWS`).
pub(super) const ACTIONS: &[Action<App>] = &[
    Action::new("hidden", toggle_hidden)
        .command(HIDDEN_FILES)
        .keys(&[Keys::at(
            Stage::FileCommands,
            &[HIDDEN_FILES.in_map(Map::Default)],
        )]),
    Action::new("refresh", refresh).command(REFRESH).keys(&[
        Keys::at(Stage::FileCommands, &[REFRESH.ignoring(SHORTCUT_MODIFIERS)]),
        Keys::at(Stage::AfterYield, &[REFRESH.ignoring(Modifiers::empty())]),
    ]),
    Action::new("open", open).command(ENTER),
    Action::new("select-all", select_all).command(SELECT_ALL),
    Action::new("history-back", back)
        .command(HISTORY_BACK)
        .keys(&[Keys::at(
            Stage::ItemNavigation,
            &[HISTORY_BACK.in_map(Map::Default).ignoring(SHIFT)],
        )]),
    Action::new("history-forward", forward)
        .command(HISTORY_FORWARD)
        .keys(&[Keys::at(
            Stage::ItemNavigation,
            &[HISTORY_FORWARD.in_map(Map::Default).ignoring(SHIFT)],
        )]),
    Action::new("parent-folder", parent)
        .command(PARENT_FOLDER)
        .keys(&[Keys::at(
            Stage::ItemNavigation,
            &[PARENT_FOLDER.ignoring(SHIFT)],
        )]),
    Action::new("navigate-up", up).keys(&[Keys {
        stage: Stage::ItemNavigation,
        reads: Reads::Press(backspace),
        when: from_the_listing,
    }]),
    Action::new("home-folder", home)
        .command(HOME_FOLDER)
        .keys(&[Keys::at(
            Stage::ItemNavigation,
            &[HOME_FOLDER.ignoring(SHIFT)],
        )]),
    views::SHOW_COLUMNS,
    views::SHOW_ICONS,
    views::SHOW_LIST,
    text_size::LARGER,
    text_size::SMALLER,
    text_size::RESET,
];
