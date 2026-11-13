// SPDX-License-Identifier: MIT

//! The location bar (A5): C6's [`LocationBar`] in the header's middle, naming the folder the
//! shown view shows, as GTK's `location_bar` does (`src/ui/browser/view_setup.rs`, with
//! `location.rs` and `location/breadcrumbs.rs` beside it).
//!
//! - **The folder it names** is GTK's `displayed_location`: the deepest column in Columns,
//!   the browser's active folder in Icons and List. The trail starts at `~` under the home
//!   folder ([`trail`]), and the entry opens on the folder's path.
//! - **Ctrl+L** opens the entry with its text selected, read where GTK's dispatcher reads it
//!   (`filter_and_location_commands`); again while editing, it keeps the typed text. While the
//!   entry has the keyboard the bar is the window's location owner ([`Holds::Location`]).
//! - **Enter or Navigate** opens the typed location (`navigate_input_located`), and **Escape,
//!   Cancel or a press elsewhere** goes back to the crumbs. The keyboard goes back to the shown
//!   view, except after a press elsewhere, which keeps it where the press put it.
//!   **A crumb** opens its folder selecting the one the user came from.
//! - **The names** are GTK's, with the keys as they are bound now ([`labels`]).
//!
//! A typed location that cannot be opened is reported on the window's error dialog
//! ([`crate::open_failed`]).
//!
//! Not on this interface yet: path suggestions (an empty list answers each request),
//! mounting a typed location that needs it, Copy path,
//! a crumb's middle click (tabs and windows), drops on crumbs, and following the window's
//! activation. `docs/rune/parity/packages/A5.md`, "Open", says who brings each.

use rune::{El, Tx};
use strata_components::location_bar::{LocationBar, LocationBarState, Trail};
use strata_core::model::Location;
use strata_desktop::keys::{Modifiers, bindings::LOCATION};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, Outcome, Stage},
    shell,
    slots::{Frame, Listener, Part, Place, Setup},
};

mod intents;
mod labels;
mod trail;

#[cfg(test)]
mod tests;

/// The element key of the bar.
const KEY: &str = "location";

pub(crate) struct Bar {
    state: LocationBarState,
    trail: Trail,
    /// The folder each crumb opens, in the trail's order: a crumb's id is its index here.
    crumbs: Vec<Location>,
    /// The folder the trail names.
    shown: Option<Location>,
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let bar = &app.location;
    let labels = labels::current();
    let element = LocationBar::new(
        KEY,
        &app.look,
        &bar.trail,
        &bar.state,
        |app: &mut App| &mut app.location.state,
        intents::heard,
    )
    .labels(&labels)
    .build(frame.cx, frame.layer);
    Some(element)
}

/// What can change the folder the bar names: the columns, or which one is active.
fn moves(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::Reset
            | BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::ColumnAdded { .. }
            | BrowserEvent::ColumnsRelocated { .. }
            | BrowserEvent::FocusChanged { .. }
            | BrowserEvent::SelectionSetChanged { .. }
    )
}

/// GTK's `displayed_location`. Only the Columns view shows every open column; it is told
/// from the others by the saved mode it answers to.
fn displayed(app: &App) -> Option<Location> {
    let columns = shell::shown_view(app).is_some_and(|view| (view.shows)("columns"));
    if columns {
        (0..)
            .map_while(|depth| app.browser.location_at(depth))
            .last()
    } else {
        app.browser.active_location()
    }
}

/// The bar names the folder the view shows now (`sync_displayed_location`). False when it
/// already did, or when no folder shows.
fn follow(app: &mut App, tx: &mut Tx) -> bool {
    // With no folder shown, the last one's trail stays, as GTK's does.
    let Some(shown) = displayed(app) else {
        return false;
    };
    if app.location.shown.as_ref() == Some(&shown) {
        return false;
    }
    let home = Location::local(crate::launch::home_directory());
    let (trail, crumbs) = trail::of(&shown, &home);
    let bar = &mut app.location;
    bar.trail = trail;
    bar.crumbs = crumbs;
    bar.shown = Some(shown);
    let polled = bar.state.navigated();
    intents::route(app, polled, tx);
    true
}

/// Ctrl+L: the entry, on the folder's path or on what was typed (`begin_location_edit`).
fn begin(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    let text = app.location.trail.text.clone();
    let polled = app.location.state.begin_edit(&text);
    intents::route(app, polled, tx);
    Outcome::Ran
}

impl Part for Bar {
    const PLACE: Place<App> = Place::Location(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            state: LocationBarState::new(),
            trail: Trail::default(),
            crumbs: Vec::new(),
            shown: None,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if moves(event) {
                heard.then(follow);
            }
        }))
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let polled = app.location.state.tick(clock, tx);
        let changed = polled.changed;
        intents::route_ticked(app, polled, tx);
        changed
    }

    // `filter_and_location_commands` in `src/ui/window/keyboard/commands.rs`:
    // `press.is(LOCATION)`, wherever the press lands.
    const COMMANDS: &'static [Action<App>] = &[Action::new("location", begin)
        .command(LOCATION)
        .keys(&[Keys::at(
            Stage::FilterAndLocationCommands,
            &[LOCATION.ignoring(Modifiers::empty())],
        )])];

    fn keyboard(app: &App) -> Option<Holds> {
        app.location
            .state
            .entry_focused()
            .then_some(Holds::Location)
    }
}
