// SPDX-License-Identifier: MIT

use std::{collections::HashMap, path::PathBuf, rc::Rc};

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
mod roots;
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
    /// Where it searches, read as it opens ([`roots`]).
    roots: Vec<PathBuf>,
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
            roots: Vec::new(),
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
    let mut palette = SearchPalette::new(
        KEY,
        |app: &App| &app.palette.state,
        |app: &mut App| &mut app.palette.state,
    )
    .new_tab_keys(Vec::new())
    .middle_click_opens(false)
    .reduced_motion(frame.cx.is_reduced_motion())
    .on_intent(heard);
    if let Some(return_to) = shell::view_focus(app) {
        palette = palette.return_focus_on_open(return_to);
    }
    palette.build(&app.look, app, frame.layer);
    None
}

fn toggle(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if app.palette.menu.is_open() {
        return Outcome::Ran;
    }
    if !app.palette.state.is_open() {
        app.palette.roots = roots::global_search_roots();
    }
    let mode = search::mode(&app.palette.roots);
    let polled = app.palette.state.toggle(mode);
    route(app, polled, tx);
    Outcome::Ran
}

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
