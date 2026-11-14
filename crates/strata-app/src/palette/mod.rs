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
//!
//! Not yet: the results' thumbnails, the header button's pressed look while it is open, and
//! Ctrl+Shift+K's folder jump. `docs/rune/parity/packages/A10.md` keeps the list.

use std::collections::HashMap;

use rune::{El, Tx};
use strata_components::{
    context_menu::ContextMenuState,
    search_palette::{By, PaletteIntent, PaletteState, Polled, SearchPalette},
};
use strata_desktop::keys::{Modifiers, bindings::SEARCH};
use strata_fs::services::SearchItem;

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, Outcome, Situation, Stage},
    filter::session::Session,
    shell,
    slots::{Frame, Later, Part, Place, Setup},
};

mod act;
pub(crate) mod menu;
mod search;

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
        }
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
    const COMMANDS: &'static [Action<App>] =
        &[Action::new("search", toggle).command(SEARCH).keys(&[
            Keys::at(
                Stage::WindowCommands,
                &[SEARCH.ignoring(Modifiers::empty())],
            )
            .when(no_text),
            Keys::at(Stage::AfterYield, &[SEARCH.ignoring(Modifiers::empty())]),
        ])];

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

fn route(app: &mut App, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

/// What the palette asks.
fn heard(app: &mut App, intent: PaletteIntent, tx: &mut Tx) {
    match intent {
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
