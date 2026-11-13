// SPDX-License-Identifier: MIT

//! What assistive technology hears of the bar, in the user's language and with the keys as
//! they are bound now (`describe_location_controls` in `src/ui/accessibility.rs`, through
//! `with_keys` in `src/ui/shortcut_reference.rs`). Built for each run of the view, so a
//! rebinding shows at once, as GTK's `follow_keys` does.

use strata_components::location_bar::LocationLabels;
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Keys,
    bindings::{ENTER, ESCAPE, LOCATION},
};

pub(super) fn current() -> LocationLabels {
    LocationLabels {
        // GTK names neither the bar, nor the crumbs' box, nor the suggestions' list.
        bar: String::new(),
        crumbs: String::new(),
        entry: with_keys(
            "Location (%{keys})",
            Some("Location"),
            Keys::One(LOCATION.shown()),
        ),
        placeholder: tr("Enter a path or URI…"),
        navigate: with_keys(
            "Navigate (%{keys})",
            Some("Navigate"),
            Keys::One(ENTER.shown()),
        ),
        cancel: with_keys(
            "Cancel (%{keys})",
            Some("Cancel"),
            Keys::One(ESCAPE.shown()),
        ),
        copy: tr("Copy path"),
        copied: tr("Path copied"),
        suggestions: String::new(),
    }
}

/// `message` with `keys` as they are bound now, or `keyless` once nothing binds them (GTK's
/// `with_keys` and its `KEYLESS` table, in `src/ui/shortcut_reference.rs`): a message with no
/// keyless form says no keys.
pub(crate) fn with_keys(message: &str, keyless: Option<&str>, keys: Keys) -> String {
    let label = keys::label(&keys);
    if let Some(keyless) = keyless.filter(|_| label.is_empty()) {
        return tr(keyless);
    }
    rust_i18n::t!(message, keys = label).into_owned()
}
