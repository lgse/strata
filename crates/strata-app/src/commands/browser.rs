// SPDX-License-Identifier: MIT

//! The window's own commands: what the browser does at a key, a menu row or a button, with
//! no part of the interface between. Each is read at the stage, and with the matcher, that
//! the GTK window's dispatcher has for it.

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
fn depth(app: &App, on: On) -> Option<usize> {
    match on {
        On::Row((depth, _)) | On::Column(depth) => Some(depth),
        On::Focus => app.browser.active_depth(),
    }
}

/// `browser_commands` in `src/ui/window/keyboard/commands.rs`, and the pane menu's row.
fn toggle_hidden(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    app.browser.toggle_hidden();
    Outcome::Ran
}

/// F5 reads every column again (`refresh_browser`); the pane menu's row reads its own.
fn refresh(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    match call.on {
        On::Row((depth, _)) | On::Column(depth) => app.browser.retry_column(depth),
        On::Focus => app.browser.refresh_all(),
    }
    Outcome::Ran
}

/// `open_context_entry` in `src/ui/browser/context_menu/item_menu.rs`: the row's entry
/// opens as a click on it does. Return on the cursor's entry is the view's own key.
fn open(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let (On::Row(row), Some(view)) = (call.on, shell::shown_view(app)) else {
        return Outcome::Declined;
    };
    (view.rows.activate)(app, row);
    Outcome::Ran
}

/// The pane menu's Select All. Ctrl+A in a view is the view's own key.
fn select_all(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let (Some(depth), Some(view)) = (depth(app, call.on), shell::shown_view(app)) else {
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

/// `navigate_up` in `src/ui/browser/view_navigation.rs`: outside Columns the parent folder;
/// in Columns the keyboard's column closes, or from the first the parent opens with the
/// folder the user came from selected.
fn up(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    let columns = shell::shown_view(app).is_some_and(|view| (view.shows)("columns"));
    let browser = app.browser.clone();
    if !columns {
        browser.parent();
        return Outcome::Ran;
    }
    // The keyboard's column: the registry has handed the browser what the view moved
    // (`call`, in `commands.rs`), so its active column is the one the keyboard is in.
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

/// BackSpace is no catalog command: `item_action` reads the bare binding, with Shift or Super
/// held too.
fn backspace(press: &KeyPress) -> bool {
    press.is(BACKSPACE.ignoring(SHIFT.union(SUPER)))
}

/// `items_unfocused_for` in `src/ui/window/keyboard/items.rs`: a key with neither Ctrl nor
/// Alt held reaches the items only from the shown view's listing.
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
    // `browser_commands` reads it with anything held; the window action `refresh` reads it
    // as bound once the chain has left the key (`WINDOW_ACTIONS` in `keyboard.rs`), which is
    // what refreshes under a menu.
    Action::new("refresh", refresh).command(REFRESH).keys(&[
        Keys::at(Stage::FileCommands, &[REFRESH.ignoring(SHORTCUT_MODIFIERS)]),
        Keys::at(Stage::AfterYield, &[REFRESH.ignoring(Modifiers::empty())]),
    ]),
    Action::new("open", open).command(ENTER),
    Action::new("select-all", select_all).command(SELECT_ALL),
    // `history_jump` in `src/ui/window/keyboard/items.rs`, which lets Shift be held.
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
    // `item_action` in `src/ui/window/keyboard/items.rs`, at `item_navigation`.
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
    // `window_commands` in `src/ui/window/keyboard/commands.rs`: the view switch.
    views::SHOW_COLUMNS,
    views::SHOW_ICONS,
    views::SHOW_LIST,
    // `zoom_shortcut` in `src/ui/window/keyboard.rs`: the text size.
    text_size::LARGER,
    text_size::SMALLER,
    text_size::RESET,
];
