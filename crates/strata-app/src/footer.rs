// SPDX-License-Identifier: MIT

//! The window's footer: C19's [`Footer`] under the view, with GTK's F1 Shortcuts button at
//! its left (`reference_toggle` in `src/ui/shortcut_footer/build.rs`). The button is named
//! and described with the reference's key as the keymap binds it, shows pressed while the
//! reference is open, and toggles it ([`crate::shortcuts`]). "Show keybinding hints" off
//! hides it, and F1 still opens the reference. At its right the active column's item and
//! selection count ([`count`]); with neither to show, the footer goes.
//!
//! Left out, with what brings each back: the clipboard badge, [A12]'s; the 10X pill, the
//! marks and the prompt, [A11]'s.
//!
//! [A11]: ../../../docs/rune/parity/packages/A11.md
//! [A12]: ../../../docs/rune/parity/packages/A12.md

use rune::{El, Tx};
use strata_components::footer::{Footer as Bar, FooterLabels};
use strata_desktop::keys::{Keys, bindings::REFERENCE};

use crate::{
    App, buttons,
    location::labels::with_keys,
    shell,
    slots::{Frame, Part, Place, Setup},
};

mod count;
#[cfg(test)]
mod tests;

pub(crate) struct Footer {
    counted: count::Counted,
}

impl Part for Footer {
    const PLACE: Place<App> = Place::Footer(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            counted: count::Counted::default(),
        }
    }

    fn poll(app: &mut App, _: f64, _: &mut Tx) -> bool {
        count::poll(app)
    }
}

/// The button's key: C19's `Footer::shortcuts_key` for the footer keyed `footer`.
const SHORTCUTS: &str = "footer-shortcuts";

fn build(app: &App, _: &mut Frame<'_, App>) -> Option<El> {
    let shows = shell::preferences(app).show_keybinding_hints;
    let footer = Bar::new("footer")
        .shortcuts(shows.then_some((
            |app: &App| &app.shortcuts.reference,
            |app: &mut App| &mut app.shortcuts.reference,
        )))
        .count(count::shown(app))
        .labels(labels());
    debug_assert_eq!(footer.shortcuts_key(), SHORTCUTS);
    // Return presses the button, as on GTK (Rune packet entry 103).
    footer
        .build(app, &app.look)
        .map(|footer| buttons::on_return_within(SHORTCUTS, footer))
}

/// The button's name and description, as GTK's `follow_keys` writes them.
fn labels() -> FooterLabels {
    let reference = Keys::One(REFERENCE.alternate(0).shown());
    FooterLabels {
        shortcuts: with_keys("%{keys} Shortcuts", Some("Shortcuts"), reference),
        shortcuts_description: with_keys(
            "Show all file-view shortcuts (%{keys})",
            Some("Show all file-view shortcuts"),
            reference,
        ),
        ..FooterLabels::default()
    }
}
