// SPDX-License-Identifier: MIT

//! What assistive technology hears of the bar, in the user's language and with the keys as
//! they are bound now (`describe_location_controls` in `src/ui/accessibility.rs`, through
//! `with_keys` in `src/ui/shortcut_reference.rs`). Built for each run of the view, so a
//! rebinding shows at once, as GTK's `follow_keys` does.

use strata_components::location_bar::LocationLabels;
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Command, Keys,
    bindings::{ENTER, ESCAPE, LOCATION},
};

pub(super) fn current() -> LocationLabels {
    LocationLabels {
        // GTK names neither the bar, nor the crumbs' box, nor the suggestions' list.
        bar: String::new(),
        crumbs: String::new(),
        entry: with_keys("Location (%{keys})", "Location", LOCATION),
        placeholder: tr("Enter a path or URI…"),
        navigate: with_keys("Navigate (%{keys})", "Navigate", ENTER),
        cancel: with_keys("Cancel (%{keys})", "Cancel", ESCAPE),
        copy: tr("Copy path"),
        copied: tr("Path copied"),
        suggestions: String::new(),
    }
}

/// `message` with `command`'s keys, or `keyless` once nothing binds it (GTK's `KEYLESS`).
fn with_keys(message: &str, keyless: &str, command: Command) -> String {
    let label = keys::label(&Keys::One(command.shown()));
    if label.is_empty() {
        return tr(keyless);
    }
    rust_i18n::t!(message, keys = label).into_owned()
}
