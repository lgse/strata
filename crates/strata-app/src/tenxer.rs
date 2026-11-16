// SPDX-License-Identifier: MIT

//! 10xer mode (A11): modal keyboard navigation, footer indicators, chord prefix dispatch and
//! footer prompts, ported from `src/ui/tenxer_mode.rs`, `src/ui/window/keyboard/tenxer.rs` and
//! `src/ui/shortcut_footer/`.

use rune::{Cx, El, Tx};
use strata_components::{
    collection::keyboard::{Command as Nav, KeyHandle, RangeKind},
    filter_bar::{
        Flash,
        chord::{self, ChordMap, ChordOption, ChordState},
        flash,
        prompt::{FooterPrompt, PromptState},
    },
};
use strata_core::i18n::tr;
use strata_desktop::keys::{self as key_catalog, Keys, bindings};

use crate::{
    App,
    commands::{Action, Holds},
    shell,
    slots::{Frame, Part, Place, Setup},
};

mod keys;
#[cfg(test)]
mod tests;

const FLASH_KEY: &str = "footer-flash";
const CHORD_KEY: &str = "footer-chord";
const PROMPT_KEY: &str = "prompt";

/// A pending two-key command (`Chord` in `src/ui/tenxer_mode.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChordKind {
    Go,
}

impl ChordKind {
    pub(crate) fn mark(self) -> String {
        let prefix = match self {
            Self::Go => Keys::One(bindings::GO.shown()),
        };
        format!("{}-", key_catalog::label(&prefix))
    }
}

pub(crate) struct Tenxer {
    pub(crate) flash: Flash,
    pub(crate) chord: ChordState<()>,
    pub(crate) armed: Option<ChordKind>,
    pub(crate) prompt: PromptState<()>,
    was_enabled: bool,
}

impl Part for Tenxer {
    const PLACE: Place<App> = Place::Overlays(build);

    const COMMANDS: &'static [Action<App>] = keys::COMMANDS;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            flash: Flash::new(),
            chord: ChordState::new(),
            armed: None,
            prompt: PromptState::new(),
            was_enabled: setup.settings.tenxer_mode,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        (app.tenxer.prompt.is_open() && app.tenxer.prompt.edit.focused).then_some(Holds::Field)
    }

    fn tab_hidden(app: &mut App, _: &mut Tx) {
        cancel_chord(app);
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let enabled = shell::preferences(app).tenxer_mode;
        let mut changed = false;
        if app.tenxer.was_enabled && !enabled {
            end_session(app);
            changed = true;
        }
        app.tenxer.was_enabled = enabled;
        let polled = app.tenxer.prompt.tick();
        changed |= polled.changed;
        prompt_builder(app).route(app, polled, tx);
        changed |= app.tenxer.flash.tick(clock, tx);
        changed
    }
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let has_options = app
        .tenxer
        .chord
        .armed()
        .is_some_and(|map| !map.options.is_empty());
    if has_options && let Some(panel) = chord::panel(&app.look, CHORD_KEY, &app.tenxer.chord) {
        frame.layer.push(panel);
    }
    None
}

fn prompt_builder(app: &App) -> FooterPrompt<App, ()> {
    let builder = FooterPrompt::new(
        PROMPT_KEY,
        |app: &App| &app.tenxer.prompt,
        |app: &mut App| &mut app.tenxer.prompt,
    );
    match shell::view_focus(app) {
        Some(focus) => builder.focus_return(focus),
        None => builder,
    }
}

/// Cancels any armed chord on `app`.
pub(crate) fn cancel_chord(app: &mut App) -> bool {
    app.tenxer.armed = None;
    app.tenxer.chord.clear()
}

/// Arms `kind` with `options` (`(keycap, label)`), filtering out unbound keycaps.
pub(crate) fn arm_chord(app: &mut App, kind: ChordKind, rows: Vec<(String, String)>) {
    let options = rows
        .into_iter()
        .filter(|(cap, _)| !cap.is_empty())
        .map(|(cap, label)| ChordOption::mention(cap, label))
        .collect();
    let mark = kind.mark();
    app.tenxer.armed = Some(kind);
    app.tenxer.chord.arm(ChordMap {
        name: mark.clone(),
        mark,
        options,
    });
}

/// Ends an active 10xer session when leaving 10xer mode (`end_tenxer_session`).
pub(crate) fn end_session(app: &mut App) {
    cancel_chord(app);
    let _ = app.tenxer.prompt.close();
    end_all_ranges(app);
}

/// Shows `"Nothing to select"` in the footer flash.
pub(crate) fn flash_nothing_to_select(app: &mut App, tx: &mut Tx) {
    app.tenxer.flash.show(tr("Nothing to select"), tx);
}

/// The `KeyHandle` of whichever collection currently shows.
pub(crate) fn active_key_handle(app: &App) -> Option<KeyHandle> {
    if app.filter.pane.shows() {
        return Some(app.filter.pane.keys_handle.clone());
    }
    let view = shell::shown_view(app)?;
    if (view.shows)("columns") {
        app.columns.active_key_handle()
    } else if (view.shows)("icons") {
        Some(app.icons.single.keys_handle.clone())
    } else if (view.shows)("list") {
        Some(app.list.single.keys_handle.clone())
    } else {
        None
    }
}

/// Queues `command` onto the shown collection's [`KeyHandle`] and ticks that collection at
/// once, so `commands::call`'s `settle` publishes the updated cursor and selection in the
/// same frame.
pub(crate) fn run_collection_command(app: &mut App, command: Nav, tx: &mut Tx) -> bool {
    let Some(handle) = active_key_handle(app) else {
        return false;
    };
    handle.run(command);
    tick_shown_collection(app, tx);
    true
}

fn tick_shown_collection(app: &mut App, tx: &mut Tx) {
    let Some(view) = shell::shown_view(app) else {
        return;
    };
    if (view.shows)("columns") {
        let depth = app.columns.active_depth();
        if let Some(collection) = crate::columns::view(app).column_collection(app, depth) {
            collection.tick(app, tx);
        }
    } else if (view.shows)("icons") {
        crate::icons::tick_active(app, tx);
    } else if (view.shows)("list") {
        crate::list::tick_active(app, tx);
    }
}

/// The active `VISUAL` or `UNSET` range for the footer.
pub(crate) fn active_range(app: &App) -> Option<RangeKind> {
    if !shell::preferences(app).tenxer_mode {
        return None;
    }
    active_key_handle(app)?.range()
}

/// Ends any active visual range on the currently shown results or listing.
pub(crate) fn end_active_range(app: &App) -> bool {
    active_key_handle(app).is_some_and(|handle| handle.end_range())
}

/// Ends any active visual range across every view and filter pane.
pub(crate) fn end_all_ranges(app: &App) -> bool {
    app.filter.pane.keys_handle.end_range()
        | app.columns.end_ranges()
        | app.icons.single.keys_handle.end_range()
        | app.list.single.keys_handle.end_range()
}

pub(crate) fn flash_el(app: &App) -> Option<El> {
    flash(&app.look, FLASH_KEY, &app.tenxer.flash)
}

pub(crate) fn chord_pill_el(app: &App) -> Option<El> {
    chord::pill(&app.look, CHORD_KEY, &app.tenxer.chord)
}

pub(crate) fn prompt_el(app: &App, cx: &mut Cx<App>, layer: &mut Vec<El>) -> Option<El> {
    prompt_builder(app).build(app, &app.look, cx, layer)
}
