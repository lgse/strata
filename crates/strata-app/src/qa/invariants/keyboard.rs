// SPDX-License-Identifier: MIT

//! `keyboard`: who has the keyboard against what the tree shows. The window's commands wait
//! while a part says it holds the keyboard ([`Holds`]), so a hold that outlives its surface
//! leaves every window shortcut dead, and a surface that takes keys with no hold lets them
//! through: Delete would move files to the trash while a name is typed. And an element must
//! have the keyboard, or the next key goes nowhere.
//!
//! - Something in the tree has the keyboard.
//! - A part holds it for a menu exactly while the tree shows a menu, for a modal layer
//!   exactly while it shows a dialog, and for an inline edit or the location bar only while a
//!   text field has the keyboard.
//! - A text field with the keyboard is held for by some part.
//!
//! A plain popover's hold ([`Holds::Popover`]) is not held to anything: the snapshot names no
//! popover.

use strata_qa::Observation;

use super::Failures;
use crate::{App, commands::Holds, shell};

/// What the tree shows that takes the keyboard.
pub(super) struct Shown<'a> {
    pub(super) observation: &'a Observation,
    /// Whether an element of the tree has the keyboard.
    pub(super) focused: bool,
}

pub(super) fn check(app: &App, shown: &Shown<'_>, failures: &mut Failures) {
    if !shown.focused {
        failures.fail("keyboard", "no element has the keyboard".to_owned());
    }
    let holds: Vec<(Holds, &str)> = shell::parts(app)
        .active()
        .filter_map(|part| Some(((part.keyboard)(app)?, part.name)))
        .collect();
    let observation = shown.observation;
    let typing = observation
        .field
        .as_ref()
        .is_some_and(|field| field.focused);
    surface(&holds, Surface::Menu, observation.menu.is_some(), failures);
    surface(
        &holds,
        Surface::Dialog,
        observation.dialog.is_some(),
        failures,
    );
    for (hold, part) in &holds {
        if matches!(hold, Holds::Edit | Holds::Location) && !typing {
            failures.fail(
                "keyboard",
                format!("{part} holds the keyboard for a text field, and none has it"),
            );
        }
    }
    if typing && !holds.iter().any(|(hold, _)| is_typing(*hold)) {
        failures.fail(
            "keyboard",
            "a text field has the keyboard, and no part holds it".to_owned(),
        );
    }
}

#[derive(Clone, Copy)]
enum Surface {
    Menu,
    Dialog,
}

impl Surface {
    fn word(self) -> &'static str {
        match self {
            Self::Menu => "a menu",
            Self::Dialog => "a dialog",
        }
    }

    fn held_by(self, hold: Holds) -> bool {
        match self {
            Self::Menu => hold == Holds::Menu,
            Self::Dialog => matches!(hold, Holds::Modal { .. }),
        }
    }
}

/// A hold for `surface` exactly while the tree shows one.
fn surface(holds: &[(Holds, &str)], surface: Surface, open: bool, failures: &mut Failures) {
    let held = holds.iter().find(|(hold, _)| surface.held_by(*hold));
    let word = surface.word();
    match (held, open) {
        (Some((_, part)), false) => failures.fail(
            "keyboard",
            format!("{part} holds the keyboard for {word}, and the tree shows none"),
        ),
        (None, true) => failures.fail(
            "keyboard",
            format!("the tree shows {word}, and no part holds the keyboard for it"),
        ),
        _ => {}
    }
}

/// Whether a hold is for typing into a text field.
fn is_typing(hold: Holds) -> bool {
    matches!(
        hold,
        Holds::Field | Holds::Filter | Holds::Location | Holds::Edit | Holds::Modal { field: true }
    )
}
