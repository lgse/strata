// SPDX-License-Identifier: MIT

use rune::Icon;

use super::{IconName, IconSet};

pub(super) mod catalog;
/// The generator `build.rs` runs, compiled here only for its tests.
#[cfg(test)]
mod generate;
mod semantic;

/// Strata's Theme colors: every icon in the element's colour.
pub const SET: IconSet = IconSet {
    name: "lucide",
    icon,
    named,
    all: &catalog::LUCIDE,
};

fn icon(name: IconName) -> &'static Icon {
    &catalog::LUCIDE[semantic::index(name)]
}

fn named(key: &str) -> Option<&'static Icon> {
    catalog::find(&catalog::LUCIDE, key, icon)
}

/// The catalogs' icon for `name`, by its index in [`catalog::LUCIDE`] and its flavours.
pub(super) const fn index(name: IconName) -> usize {
    semantic::index(name)
}
