// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{El, Tx};
use strata_components::footer::{Footer as Bar, FooterLabels};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Keys,
    bindings::{PASTE, REFERENCE},
};

use crate::{
    App, clipboard, filter,
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

    fn poll(app: &mut App, clock: f64, _: &mut Tx) -> bool {
        count::poll(app, clock)
    }
}

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
    footer.build(app, &app.look)
}

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
