// SPDX-License-Identifier: MIT

//! The default map's arrows in the sidebar, as GTK's dispatcher reads them at its
//! `focus_navigation` stage (`sidebar_navigation` in `src/ui/window/keyboard/focus.rs`): with
//! the keyboard in the sidebar and neither Ctrl nor Alt held, an arrow (Shift and Super
//! tolerated, `FOCUS_MODIFIERS`) or its other key as pressed (`h`, `j`, `k`, `l`) moves it.
//!
//! - **Right** gives it back to the files (`SidebarFocus::restore`, which falls back to
//!   `browser.focus_active`).
//! - **Up** without Shift, from the sidebar's first element, goes up to the header's toggle
//!   (`TopBarNavigation::move_up_from_sidebar`).
//! - **Up and Down** otherwise move between the rows and headings, which Rune's own focus
//!   navigation does for the plain arrows; their other keys, and the arrows with Shift or
//!   Super, are handed to it as the plain arrows. Shift+Up on the first element moves nothing.
//! - **Left** moves nothing, and is taken, as GTK's `child_focus(Left)` finds nothing.
//!
//! In 10xer mode GTK reads none of this (`tenxer_pane_focus`), and C8 takes `h`, `j`, `k`, `l`,
//! Left and Backspace itself.

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

/// GTK's `FOCUS_MODIFIERS` (`src/ui/focus_navigation.rs`).
const FOCUS_MODIFIERS: Modifiers = Modifiers::SHIFT_MASK.union(Modifiers::SUPER_MASK);

/// The header's toggle, which Up from the sidebar's top reaches.
pub(super) const TOGGLE: &str = "header-sidebar-toggle";

/// The pinned section's heading, the first element when no place is shown.
pub(super) const PINNED_HEADING: &str = "sidebar-heading-pinned";

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

/// GTK's guard: the keyboard is in the sidebar, outside 10xer mode.
fn in_sidebar(app: &App, at: &Situation) -> bool {
    at.within(KEY) && !shell::preferences(app).tenxer_mode
}

fn left(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    Outcome::Ran
}

fn down(_: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    step(call, DOWN, Key::Down, tx)
}

/// GTK's `!event.shift()` sends Up to the header (`move_up_from_sidebar`) only without Shift;
/// with it, `child_focus(Up)` finds nothing above the first element.
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

/// A move between the sidebar's elements, as GTK's `child_focus` makes it with Shift or Super
/// held too: Rune's focus navigation takes the plain arrow itself, refuses it with Super and
/// does nothing with Shift, so any other press is handed to it as the plain arrow.
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
    /// The key of the sidebar's first element: the first place's row, else the pinned
    /// section's heading.
    pub(super) fn first_key(&self) -> Option<String> {
        let place = self
            .listed
            .rows
            .iter()
            .find(|row| matches!(row.item, ItemId::Place(_)));
        match place {
            Some(row) => Some(row.item.key(KEY)),
            None => (!self.listed.pins.is_empty()).then(|| PINNED_HEADING.to_owned()),
        }
    }
}
