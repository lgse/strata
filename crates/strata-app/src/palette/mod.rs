// SPDX-License-Identifier: MIT

//! The search palette (A10): C17's [`SearchPalette`] for Ctrl+K and the header's Search
//! button, as GTK's `SearchDialog` (`src/ui/search.rs`, `src/ui/search/panel.rs`,
//! `src/ui/search/results.rs`) and its wiring (`src/ui/window/composition/search.rs`).
//!
//! - **Opening and closing.** The `search` command toggles it: Ctrl+K where no text field has
//!   the keyboard (`window_commands`), and from anywhere once the chain has left the key
//!   (`after_yield`), which is how Ctrl+K closes it from its own field. The header's Search
//!   button runs the same command. Escape, a click outside and the toggle close it, and the
//!   keyboard goes back to what opened it. While open it is the window's modal layer
//!   ([`Holds::Modal`]), its field typing.
//! - **What it searches** ([`search`]): strata-fs's index of Home, with every subfolder but
//!   the excluded ones, hidden entries as the browser shows them and the search limits from
//!   the settings, started as it opens. Each edit asks the index again, and a batch counts
//!   only for the text the field holds. The mounted local drives GTK adds after Home are a
//!   follow-up (`docs/rune/parity/packages/A10.md`, "Open").
//! - **Opening a result** ([`act`]): Enter or a click. A folder is navigated to; a file is
//!   revealed in its folder, selected, and opened first when "Open files directly" is on. The
//!   preview drawer GTK shows a file in otherwise is A15's.
//! - **Its folder:** Alt+Return, with Super held or not, or the result's menu ([`menu`], "Open
//!   containing folder") reveals it in its folder, the folder's filter emptied first, as GTK's
//!   `reveal_location` does. The palette's field reads Alt+Return itself (C17's reveal
//!   chord): Rune's single-line field takes Return with any modifier, so the window's
//!   commands never hear it there. A keymap that binds `open_containing_folder` to another
//!   key is not followed yet, as GTK's is (finding lgse/strata#2470).
//! - **No new tab:** GTK's palette takes neither Ctrl+Return nor a middle click (driven), so
//!   the palette is given no new-tab chord and its middle click is off.
//! - **The folder jump** ([`jump`]): the `jump_to_folder` command toggles the palette on the
//!   folders visited before, as GTK's `jump-folder` action: Ctrl+Shift+K where no text field
//!   has the keyboard (`window_commands` and `direct_commands`) and once the chain has left
//!   the key (`after_yield`). Either command closes the palette whichever mode it is open in.
//!   The history is strata-fs's [`NavigationHistory`], which each tab's browser fills with
//!   the local folders its columns finish listing ([`visits`]). A result opens and reveals as
//!   in Ctrl+K's mode.
//!
//! Not yet: the results' thumbnails and the header button's pressed look while it is open.
//! `docs/rune/parity/packages/A10.md` keeps the list.

use std::{collections::HashMap, rc::Rc};

use rune::{El, Tx};
use strata_components::{
    context_menu::ContextMenuState,
    search_palette::{By, PaletteIntent, PaletteState, Polled, SearchPalette},
};
use strata_desktop::keys::{
    Map, Modifiers,
    bindings::{JUMP_TO_FOLDER, SEARCH},
};
use strata_fs::services::{NavigationHistory, SearchItem};

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, Outcome, Situation, Stage},
    filter::session::Session,
    shell,
    slots::{Frame, Later, Listener, Part, Place, Setup},
};

mod act;
mod jump;
pub(crate) mod menu;
mod search;
mod visits;

#[cfg(test)]
mod tests;

/// The palette's key: its dialog is `palette-dialog`, its field `palette-field`.
const KEY: &str = "palette";

pub(crate) struct Palette {
    state: PaletteState,
    session: Session,
    /// The palette's generation of the query the field holds, for the answers.
    asked: u64,
    /// Each result shown, by its id: its path.
    found: HashMap<String, SearchItem>,
    /// The result's menu ([`menu`]).
    menu: ContextMenuState,
    later: Later<App>,
    /// The folders visited, for the jump: the process's one history.
    history: Rc<NavigationHistory>,
    /// This tab's columns whose folders have not finished listing ([`visits`]).
    visits: visits::Visits,
}

impl Part for Palette {
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            state: PaletteState::new(),
            session: Session::default(),
            asked: 0,
            found: HashMap::new(),
            menu: ContextMenuState::default(),
            later: setup.later.clone(),
            history: NavigationHistory::shared(),
            visits: visits::Visits::default(),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if let Some(visit) = visits::Visit::of(event) {
                heard.then(move |app: &mut App, _: &mut Tx| {
                    visits::heard(app, visit);
                    false
                });
            }
        }))
    }

    fn poll(app: &mut App, _clock: f64, _tx: &mut Tx) -> bool {
        if app.palette.state.is_open() {
            app.palette.state.tick();
        }
        false
    }

    // `window_commands` reads Ctrl+K where no text field has the keyboard, and the window
    // action `search` reads it once the chain has left the key (`WINDOW_ACTIONS` in
    // `src/ui/window/keyboard.rs`).
    //
    // GTK reads Ctrl+Shift+K at `window_commands` in the default map's binding, at
    // `direct_commands` in the active map's, and as the window action `jump-folder` once the
    // chain has left the key (`src/ui/window/keyboard/commands.rs`, `direct.rs`).
    const COMMANDS: &'static [Action<App>] = &[
        Action::new("search", toggle).command(SEARCH).keys(&[
            Keys::at(
                Stage::WindowCommands,
                &[SEARCH.ignoring(Modifiers::empty())],
            )
            .when(no_text),
            Keys::at(Stage::AfterYield, &[SEARCH.ignoring(Modifiers::empty())]),
        ]),
        Action::new("jump-folder", toggle_jump)
            .command(JUMP_TO_FOLDER)
            .keys(&[
                Keys::at(
                    Stage::WindowCommands,
                    &[JUMP_TO_FOLDER.in_map(Map::Default)],
                )
                .when(no_text),
                Keys::at(
                    Stage::DirectCommands,
                    &[JUMP_TO_FOLDER.ignoring(Modifiers::empty())],
                )
                .when(no_text),
                Keys::at(
                    Stage::AfterYield,
                    &[JUMP_TO_FOLDER.ignoring(Modifiers::empty())],
                ),
            ]),
    ];

    fn keyboard(app: &App) -> Option<Holds> {
        let palette = &app.palette;
        palette.state.is_open().then_some(Holds::Modal {
            field: !palette.menu.is_open(),
        })
    }
}

fn no_text(_: &App, at: &Situation) -> bool {
    !at.text()
}

/// The palette, while open, over every other dialog.
fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let return_to = shell::view_focus(app).unwrap_or_default();
    SearchPalette::new(
        KEY,
        |app: &App| &app.palette.state,
        |app: &mut App| &mut app.palette.state,
    )
    .new_tab_keys(Vec::new())
    .middle_click_opens(false)
    .return_focus_on_open(return_to)
    .reduced_motion(frame.cx.is_reduced_motion())
    .on_intent(heard)
    .build(&app.look, app, frame.layer);
    None
}

/// Ctrl+K or the header's button: opens the palette on Home, or closes it. While a result's
/// menu is open the key is the menu's and does nothing, as GTK's popover takes it (driven).
fn toggle(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app.palette.menu.is_open() {
        return Outcome::Ran;
    }
    let mode = search::mode();
    let polled = app.palette.state.toggle(mode);
    route(app, polled, tx);
    Outcome::Ran
}

/// Ctrl+Shift+K: opens the palette on the folders visited, or closes it. As for Ctrl+K, the
/// key does nothing while a result's menu is open.
fn toggle_jump(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app.palette.menu.is_open() {
        return Outcome::Ran;
    }
    let polled = app.palette.state.toggle(jump::mode());
    route(app, polled, tx);
    Outcome::Ran
}

fn route(app: &mut App, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

/// What the palette asks.
fn heard(app: &mut App, intent: PaletteIntent, tx: &mut Tx) {
    match intent {
        PaletteIntent::Query(query) if jump::is_open(app) => jump::asked(app, &query, tx),
        PaletteIntent::Query(query) => search::asked(app, &query),
        PaletteIntent::Open { id, .. } => act::open(app, &id, tx),
        PaletteIntent::Reveal { id } => act::reveal(app, &id, tx),
        PaletteIntent::ResultMenu { id, at } => menu::open(app, &id, at),
        PaletteIntent::Closed { by } => closed(app, by),
        _ => {}
    }
}

/// The palette closed: its search stops, and its menu goes with it.
fn closed(app: &mut App, _by: By) {
    app.palette.session.cancel();
    app.palette.found.clear();
    app.palette.menu.close();
}
