// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{El, Tx};
use strata_components::footer::reference::{ReferenceState, ShortcutData, ShortcutReference};
use strata_desktop::keys::{Keymap, Modifiers, active_keymap, bindings::REFERENCE};

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, Outcome, Situation, Stage},
    shell,
    slots::{Frame, Part, Place, Setup},
};

mod contents;
mod data;
mod sections;
mod tenxer;

pub(crate) use contents::CUSTOM_ACTION;
use contents::Mode;
pub(crate) use sections::command_sections;

pub(crate) struct Shortcuts {
    pub(crate) reference: ReferenceState,
    data: ShortcutData,
    /// What `data` was built from.
    built: Option<Built>,
}

#[derive(Clone)]
struct Built {
    keymap: Rc<Keymap>,
    mode: Mode,
    typing: bool,
    tenxer: bool,
    locale: String,
}

impl PartialEq for Built {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.keymap, &other.keymap)
            && self.mode == other.mode
            && self.typing == other.typing
            && self.tenxer == other.tenxer
            && self.locale == other.locale
    }
}

impl Part for Shortcuts {
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            reference: ReferenceState::new(),
            data: ShortcutData::default(),
            built: None,
        }
    }

    fn poll(app: &mut App, _: f64, _: &mut Tx) -> bool {
        refresh(app) && app.shortcuts.reference.is_open()
    }

    fn tab_shown(app: &mut App, _: &mut Tx) {
        refresh(app);
    }

    fn keyboard(app: &App) -> Option<Holds> {
        // The search field has the keyboard when it opens, and typing there is the field's.
        app.shortcuts
            .reference
            .is_open()
            .then_some(Holds::Modal { field: true })
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("shortcut-reference", toggle)
        .command(REFERENCE)
        .keys(&[
            Keys::at(Stage::InputOwner, &[REFERENCE.ignoring(Modifiers::empty())])
                .when(outside_rename),
        ])];
}

fn outside_rename(_: &App, at: &Situation) -> bool {
    at.holds() != Some(Holds::Edit)
}

fn toggle(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    refresh(app);
    app.shortcuts.reference.toggle();
    Outcome::Ran
}

/// Builds the rows again when what they depend on changed. True when they were.
fn refresh(app: &mut App) -> bool {
    let now = Built {
        keymap: active_keymap(),
        mode: mode(app),
        typing: shell::preferences(app).type_to_search,
        tenxer: shell::preferences(app).tenxer_mode,
        locale: rust_i18n::locale().to_string(),
    };
    let shortcuts = &mut app.shortcuts;
    if shortcuts.built.as_ref() == Some(&now) {
        return false;
    }
    let revision = shortcuts.data.revision + 1;
    shortcuts.data = data::rows(now.mode, now.tenxer, revision);
    shortcuts.built = Some(now);
    true
}

fn mode(app: &App) -> Mode {
    let shows = |mode: &str| shell::shown_view(app).is_some_and(|view| (view.shows)(mode));
    if shows("icons") {
        Mode::Icons
    } else if shows("list") {
        Mode::List
    } else {
        Mode::Columns
    }
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    if !app.shortcuts.reference.is_open() {
        return None;
    }
    ShortcutReference::new(
        "shortcuts",
        |app: &App| &app.shortcuts.reference,
        |app: &mut App| &mut app.shortcuts.reference,
        |app: &App| &app.shortcuts.data,
    )
    .labels(data::labels())
    .keys(data::panel_keys(shell::preferences(app).tenxer_mode))
    .reduced_motion(frame.cx.is_reduced_motion())
    .build(app, &app.look, frame.layer);
    None
}

#[cfg(test)]
mod tests;
