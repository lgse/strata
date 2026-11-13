// SPDX-License-Identifier: MIT

//! The default map's arrows in the window's header, as GTK's dispatcher reads them at its
//! `focus_navigation` stage (`top_bar_navigation` in `src/ui/window/keyboard/focus.rs`, through
//! `TopBarNavigation` in `src/ui/top_bar_navigation.rs`, which holds the header, the sidebar
//! and its toggle together, as this part does): with the keyboard in the header or its location
//! bar's crumbs, not in a text field, an arrow (Super tolerated) moves it.
//!
//! - **Left and Right** move along the header, as `child_focus(TabBackward)` and
//!   `(TabForward)` do: Rune's Tab order is the header's, from the sidebar's toggle to the last
//!   window button, and at either end nothing moves.
//! - **Down** goes back to the sidebar's row that Up left (`return_to_sidebar`), else the
//!   sidebar's first element, and to the files while the sidebar is hidden.
//! - **Up** is taken.
//!
//! In 10xer mode GTK reads none of this (`tenxer_pane_focus`).

use rune::{Key, KeyEvent, Mods, Tx};
use strata_components::sidebar::{ItemId, Step};
use strata_desktop::keys::{
    Modifiers,
    bindings::{DOWN, LEFT, RIGHT, UP},
};

use super::{KEY, keys::TOGGLE};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
    shell,
};

/// The location bar's elements, which GTK's header box holds beside the buttons.
const LOCATION: &str = "location";

macro_rules! arrow {
    ($id:literal, $command:expr, $run:expr) => {
        Action::new($id, $run).command($command).keys(&[Keys::at(
            Stage::FocusNavigation,
            &[$command.ignoring(Modifiers::SUPER_MASK)],
        )
        .when(in_header)])
    };
}

pub(super) const TO_LEFT: Action<App> = arrow!("header-left", LEFT, left);
pub(super) const TO_DOWN: Action<App> = arrow!("header-down", DOWN, down);
pub(super) const TO_UP: Action<App> = arrow!("header-up", UP, up);
pub(super) const TO_RIGHT: Action<App> = arrow!("header-right", RIGHT, right);

/// GTK's guard: `top_bar.has_focus()` and no text field has the keyboard, outside 10xer mode.
fn in_header(app: &App, at: &Situation) -> bool {
    let header = at.within("header") || at.within(LOCATION);
    header && !at.text() && !shell::preferences(app).tenxer_mode
}

fn left(_: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if tx.focused() != Some(TOGGLE) {
        tab(tx, true);
    }
    Outcome::Ran
}

fn right(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    if tx.focused() != Some(shell::header_last(app).as_str()) {
        tab(tx, false);
    }
    Outcome::Ran
}

fn up(_: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    Outcome::Ran
}

fn down(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    let back = app
        .sidebar
        .back_from_header()
        .or_else(|| shell::view_focus(app).map(str::to_owned));
    if let Some(key) = back {
        tx.focus(key, true);
    }
    Outcome::Ran
}

/// One step along the header, as Rune's focus navigation takes Tab and Shift+Tab.
fn tab(tx: &mut Tx, back: bool) {
    tx.send_key(KeyEvent {
        key: Key::Tab,
        mods: Mods {
            shift: back,
            ..Mods::default()
        },
    });
}

impl super::Sidebar {
    /// Where Down from the header lands in the sidebar: the row Up left, once, while it is
    /// still shown (`return_to_sidebar` takes it), else the sidebar's first element. None while
    /// the sidebar is hidden.
    fn back_from_header(&mut self) -> Option<String> {
        if self.state.width.shown() == Step::Hidden {
            return None;
        }
        let kept = self.header_entry.take().filter(|key| self.shows(key));
        kept.or_else(|| self.first_key())
    }

    /// Whether the sidebar shows the element keyed `key` now: a collapsed PINNED still shows
    /// the open place's pin.
    fn shows(&self, key: &str) -> bool {
        let row = self.listed.rows.iter().find(|row| row.item.key(KEY) == key);
        match row {
            Some(row) => {
                !matches!(row.item, ItemId::Pin(_))
                    || !self.state.pinned_collapsed
                    || self.current.as_ref() == Some(&row.item)
            }
            None => key == super::keys::PINNED_HEADING && !self.listed.pins.is_empty(),
        }
    }
}
