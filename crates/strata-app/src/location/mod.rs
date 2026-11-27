// SPDX-License-Identifier: MIT

use rune::{El, Tx};
use strata_components::location_bar::{LocationBar, LocationBarState, Trail};
use strata_core::model::Location;
use strata_desktop::keys::{Modifiers, bindings::LOCATION};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, Outcome, Stage},
    shell,
    slots::{Frame, Later, Listener, Part, Place, Setup},
};

mod completion;
mod intents;
pub(crate) mod labels;
mod trail;

#[cfg(test)]
mod tests;

pub(crate) fn window_focused(app: &mut App, focused: bool, tx: &mut Tx) {
    let polled = app.location.state.set_window_focused(focused);
    intents::route(app, polled, tx);
}

/// The element key of the bar.
const KEY: &str = "location";

pub(crate) struct Bar {
    state: LocationBarState,
    trail: Trail,
    /// The folder each crumb opens, in the trail's order: a crumb's id is its index here.
    crumbs: Vec<Location>,
    /// The folder the trail names.
    shown: Option<Location>,
    typed: Option<String>,
    /// The suggestions' listing in flight.
    completing: completion::Completing,
    /// Where a listing's answer comes back.
    later: Later<App>,
}

#[cfg(test)]
impl Bar {
    pub(crate) fn state(&self) -> &LocationBarState {
        &self.state
    }
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
    .dnd(crate::dnd::binder(app))
    .content_types(crate::dnd::CONTENT_TYPES)
    .drop_action(intents::drop_action)
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

fn renames(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::ColumnAdded { .. }
            | BrowserEvent::ColumnsRelocated { .. }
    )
}

/// False: nothing drawn changes, the entry being closed.
fn forget_typed(app: &mut App, _: &mut Tx) -> bool {
    app.location.typed = None;
    false
}

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

fn follow(app: &mut App, tx: &mut Tx) -> bool {
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
    let bar = &app.location;
    let text = bar.typed.clone().unwrap_or_else(|| bar.trail.text.clone());
    let polled = app.location.state.begin_edit(&text);
    intents::route(app, polled, tx);
    Outcome::Ran
}

impl Part for Bar {
    const PLACE: Place<App> = Place::Location(build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            state: LocationBarState::new(),
            trail: Trail::default(),
            crumbs: Vec::new(),
            shown: None,
            typed: None,
            completing: completion::Completing::default(),
            later: setup.later.clone(),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if renames(event) {
                heard.then(forget_typed);
            }
            if moves(event) {
                heard.then(follow);
            }
        }))
    }

    fn switched(app: &mut App, tx: &mut Tx) {
        app.location.typed = None;
        follow(app, tx);
    }

    fn tab_hidden(app: &mut App, tx: &mut Tx) {
        let ended = app.location.state.end_edit();
        intents::route(app, ended, tx);
        let dismissed = app.location.state.dismiss_completion();
        intents::route(app, dismissed, tx);
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        app.location
            .state
            .hold(crate::menus::field_menu_open(app, tx));
        let polled = app.location.state.tick(clock, tx);
        let changed = polled.changed;
        intents::route_ticked(app, polled, tx);
        changed
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("location", begin)
        .command(LOCATION)
        .keys(&[Keys::at(
            Stage::FilterAndLocationCommands,
            &[LOCATION.ignoring(Modifiers::empty())],
        )])];

    fn keyboard(app: &App) -> Option<Holds> {
        let state = &app.location.state;
        if state.menu().is_some() {
            return Some(Holds::Menu);
        }
        state.entry_focused().then_some(Holds::Location)
    }
}
