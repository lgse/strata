// SPDX-License-Identifier: MIT

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

fn in_header(app: &App, at: &Situation) -> bool {
    at.in_top_bar() && !at.text() && !shell::preferences(app).tenxer_mode
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
    fn back_from_header(&mut self) -> Option<String> {
        if self.state.width.shown() == Step::Hidden {
            return None;
        }
        let kept = self.header_entry.take().filter(|key| self.shows(key));
        kept.or_else(|| self.first_key())
    }

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
