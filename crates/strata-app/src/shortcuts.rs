// SPDX-License-Identifier: MIT

//! The shortcut reference, F1: C19's [`ShortcutReference`] over the window, listing the
//! default map's shortcuts as GTK's reference lists them (`src/ui/shortcut_reference.rs`,
//! `src/ui/shortcut_footer/`).
//!
//! - **Opening and closing.** F1, as the keymap binds it, opens it from anywhere but an
//!   inline rename: GTK reads it at `input_owner` (`footer_key`), past an open rename. The
//!   footer's Shortcuts button toggles it too ([`crate::footer`]). F1 again, Escape or a click
//!   on the backdrop closes it, and the keyboard goes back to what had it.
//! - **Its keys.** While it is open it is a modal layer with a text field: every key is the
//!   panel's, and no window command runs, Ctrl+, among them. Its area keys and F1 are the
//!   keymap's ([`data::panel_keys`]).
//! - **What it lists.** The shown view's navigation, then files and selection, search and
//!   tools, preview media and the custom actions a user bound, with the keys the keymap binds
//!   now: a saved keymap shows its keys, and a row with no key is left out. The rows are
//!   rebuilt when the keymap, the view, "Type to search" or the language changes.
//!
//! - **In 10xer mode** it lists 10xer's sections instead ([`tenxer`]), and a typed `~`
//!   toggles it too, as the 10xer map binds it.
//!
//! Left out, with what brings each back: the portal chooser's rows, 10xer's among them,
//! [A20]'s.
//!
//! [A20]: ../../../docs/rune/parity/packages/A20.md

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

/// What the rows depend on beside the tables: GTK rebuilds them on a keymap revision and
/// on "Type to search", and builds them for the view and in the language shown.
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

/// GTK's `preview_and_footer_owner` reads the footer's keys only while no inline edit is open.
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

/// The shown view, as GTK's `BrowserMode` names it.
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
