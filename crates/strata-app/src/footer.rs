// SPDX-License-Identifier: MIT

//! The window's footer: C19's [`Footer`] under the view, with GTK's F1 Shortcuts button at
//! its left (`reference_toggle` in `src/ui/shortcut_footer/build.rs`). The button is named
//! and described with the reference's key as the keymap binds it, shows pressed while the
//! reference is open, and toggles it ([`crate::shortcuts`]). "Show keybinding hints" off
//! hides it, and F1 still opens the reference. At its right the "Files on clipboard" badge
//! (`paste_label` in `src/ui/shortcut_footer/build.rs`, `refresh_paste_availability` in
//! `src/ui/shortcut_footer/status.rs`) while file URIs are on the clipboard
//! ([`crate::clipboard`]), and the active column's item and selection count ([`count`]);
//! with nothing to show, the footer goes.
//!
//! Left out, with what brings each back: the 10X pill, the marks and the prompt, [A11]'s.
//!
//! [A11]: ../../../docs/rune/parity/packages/A11.md

use std::rc::Rc;

use rune::{El, Tx};
use strata_components::footer::{Footer as Bar, FooterLabels};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Keys,
    bindings::{PASTE, REFERENCE},
};

use crate::{
    App, buttons, clipboard, filter,
    location::labels::with_keys,
    shell,
    slots::{Frame, Listener, Part, Place, Setup},
    tenxer,
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

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(count::listener())
    }

    fn poll(app: &mut App, _: f64, _: &mut Tx) -> bool {
        count::poll(app)
    }
}

/// The button's key: C19's `Footer::shortcuts_key` for the footer keyed `footer`.
const SHORTCUTS: &str = "footer-shortcuts";

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let prefs = shell::preferences(app);
    let shows = prefs.show_keybinding_hints;
    let prompt = tenxer::prompt_el(app, frame.cx, frame.layer);
    let jobs = prompt
        .is_none()
        .then(|| crate::custom_actions::jobs_el(app, frame.layer))
        .flatten();
    let footer = Bar::new("footer")
        .shortcuts(shows.then_some((
            |app: &App| &app.shortcuts.reference,
            |app: &mut App| &mut app.shortcuts.reference,
        )))
        .clipboard(clipboard::has_files(app))
        .tenxer(prefs.tenxer_mode)
        .mark(filter::tenxer_mark(app))
        .current_hit(filter::current_hit_path(app))
        .range(tenxer::active_range(app))
        .flash(tenxer::flash_el(app))
        .chord(tenxer::chord_pill_el(app))
        .jobs(jobs)
        .prompt(prompt)
        .count(count::shown(app))
        .labels(labels());
    debug_assert_eq!(footer.shortcuts_key(), SHORTCUTS);
    // Return presses the button, as on GTK (Rune packet entry 103).
    footer
        .build(app, &app.look)
        .map(|footer| buttons::on_return_within(SHORTCUTS, footer))
}

/// The button's, clipboard badge's and 10xer indicators' names and descriptions, as GTK's
/// `follow_keys` and `indicators.rs` write them.
fn labels() -> FooterLabels {
    let reference = Keys::One(REFERENCE.alternate(0).shown());
    let mode = tr("10xer mode");
    let caveat = tr("(experimental feature, under active development)");
    let filter_prefix = tr("filter:");
    let search_prefix = tr("search:");
    FooterLabels {
        shortcuts: with_keys("%{keys} Shortcuts", Some("Shortcuts"), reference),
        shortcuts_description: with_keys(
            "Show all file-view shortcuts (%{keys})",
            Some("Show all file-view shortcuts"),
            reference,
        ),
        clipboard: tr("Files on clipboard"),
        clipboard_description: paste_description(),
        tenxer: "10X".to_owned(),
        tenxer_name: rust_i18n::t!(
            "%{mode} (experimental feature, under active development)",
            mode = mode
        )
        .into_owned(),
        tenxer_caveat: caveat,
        filter: Rc::new(move |q| format!("{filter_prefix} {q}")),
        search: Rc::new(move |q| format!("{search_prefix} {q}")),
        visual: (tr("VISUAL"), tr("Visual select")),
        unset: (tr("UNSET"), tr("Visual unset")),
    }
}

/// GTK's `paste_label` (`src/ui/shortcut_footer/build.rs`), whose `KEYLESS` form is empty
/// when `PASTE` is unbound (`src/ui/shortcut_reference.rs`).
fn paste_description() -> String {
    let label = keys::label(&Keys::One(PASTE.shown()));
    if label.is_empty() {
        String::new()
    } else {
        rust_i18n::t!(
            "Press %{keys} to paste into a supported directory.",
            keys = label
        )
        .into_owned()
    }
}
