// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::sidebar::ItemId;
use strata_desktop::keys::{
    Command, KeyPress, Modifiers,
    bindings::{DOWN, LEFT, RIGHT, UP},
};

use super::KEY;
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
    shell,
};

const FOCUS_MODIFIERS: Modifiers = Modifiers::SHIFT_MASK.union(Modifiers::SUPER_MASK);

/// The header's toggle, which Up from the sidebar's top reaches.
pub(super) const TOGGLE: &str = "header-sidebar-toggle";

/// The pinned section's heading, the first element when no place is shown.
pub(super) const PINNED_HEADING: &str = "sidebar-heading-pinned";

/// The devices section's heading, the first element when no place or pin is shown.
pub(super) const DEVICES_HEADING: &str = "sidebar-heading-devices";

/// One arrow's keys at the stage: the arrow, and its other key as pressed.
macro_rules! arrow {
    ($id:literal, $command:expr, $run:expr) => {
        Action::new($id, $run).command($command).keys(&[Keys::at(
            Stage::FocusNavigation,
            &[$command.ignoring(FOCUS_MODIFIERS), $command.alternates()],
        )
        .when(in_sidebar)])
    };
}

pub(super) const TO_LEFT: Action<App> = arrow!("sidebar-left", LEFT, left);
pub(super) const TO_DOWN: Action<App> = arrow!("sidebar-down", DOWN, down);
pub(super) const TO_UP: Action<App> = arrow!("sidebar-up", UP, up);
pub(super) const TO_RIGHT: Action<App> = arrow!("sidebar-right", RIGHT, right);

fn in_sidebar(app: &App, at: &Situation) -> bool {
    at.within(KEY) && !shell::preferences(app).tenxer_mode
}

fn left(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    Outcome::Ran
}

fn down(_: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    step(call, DOWN, Key::Down, tx)
}

fn up(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    let shift = call
        .press
        .is_some_and(|press| press.modifiers().contains(Modifiers::SHIFT_MASK));
    let at_top = tx.focused().is_some() && tx.focused() == app.sidebar.first_key().as_deref();
    match (at_top, shift) {
        (false, _) => step(call, UP, Key::Up, tx),
        (true, true) => Outcome::Ran,
        (true, false) => {
            app.sidebar.header_entry = tx.focused().map(str::to_owned);
            tx.focus(TOGGLE, true);
            Outcome::Ran
        }
    }
}

fn right(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
    Outcome::Ran
}

fn step(call: &Call<'_>, arrow: Command, key: Key, tx: &mut Tx) -> Outcome {
    if call.press.is_none_or(|press| plain(press, arrow)) {
        return Outcome::Declined;
    }
    tx.send_key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    Outcome::Ran
}

/// The arrow itself, with nothing held.
fn plain(press: &KeyPress, arrow: Command) -> bool {
    press.modifiers().is_empty() && !press.is(arrow.alternates())
}

impl super::Sidebar {
    pub(super) fn first_key(&self) -> Option<String> {
        let place = self
            .listed
            .rows
            .iter()
            .find(|row| matches!(row.item, ItemId::Place(_)));
        if let Some(row) = place {
            return Some(row.item.key(KEY));
        }
        if !self.listed.pins.is_empty() {
            return Some(PINNED_HEADING.to_owned());
        }
        (!self.devices.shown().is_empty()).then(|| DEVICES_HEADING.to_owned())
    }
}
