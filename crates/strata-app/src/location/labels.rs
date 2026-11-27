// SPDX-License-Identifier: MIT

use strata_components::location_bar::LocationLabels;
use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Keys,
    bindings::{ENTER, ESCAPE, LOCATION},
};

pub(super) fn current() -> LocationLabels {
    LocationLabels {
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

pub(crate) fn with_keys(message: &str, keyless: Option<&str>, keys: Keys) -> String {
    let label = keys::label(&keys);
    if let Some(keyless) = keyless.filter(|_| label.is_empty()) {
        return tr(keyless);
    }
    rust_i18n::t!(message, keys = label).into_owned()
}
