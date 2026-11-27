// SPDX-License-Identifier: MIT

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
    // A floating chooser in front has the keyboard, in a window of its own.
    if !shown.focused && !crate::chooser::floating::is_open(app) {
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
    // A chooser's footer error is an alert the snapshot reads as its dialog; it takes no keys.
    let dialog = observation
        .dialog
        .as_deref()
        .is_some_and(|name| !crate::chooser::shows_error(app, name));
    surface(&holds, Surface::Dialog, dialog, failures);
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

    /// Whether `hold` holds the keyboard for this surface while it shows.
    fn held_by(self, hold: Holds) -> bool {
        match self {
            Self::Menu => matches!(hold, Holds::Menu | Holds::Popover),
            Self::Dialog => matches!(hold, Holds::Modal { .. } | Holds::Popover),
        }
    }

    /// Whether `hold` says this surface shows: a popover may show as no dialog.
    fn shown_by(self, hold: Holds) -> bool {
        match self {
            Self::Menu => hold == Holds::Menu,
            Self::Dialog => matches!(hold, Holds::Modal { .. }),
        }
    }
}

/// A hold for `surface` while the tree shows one, and one shown while a hold says it shows.
fn surface(holds: &[(Holds, &str)], surface: Surface, open: bool, failures: &mut Failures) {
    let held = holds.iter().any(|(hold, _)| surface.held_by(*hold));
    let owed = holds.iter().find(|(hold, _)| surface.shown_by(*hold));
    let word = surface.word();
    match (owed, held, open) {
        (Some((_, part)), _, false) => failures.fail(
            "keyboard",
            format!("{part} holds the keyboard for {word}, and the tree shows none"),
        ),
        (_, false, true) => failures.fail(
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
        Holds::Field
            | Holds::Filter
            | Holds::Location
            | Holds::Edit
            | Holds::Prompt
            | Holds::Modal { field: true }
    )
}
