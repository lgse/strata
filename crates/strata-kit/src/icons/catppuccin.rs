// SPDX-License-Identifier: MIT

use rune::Icon;

use super::lucide::{self, catalog};
use super::{IconName, IconSet};

/// Brand colors on a dark background.
pub const MOCHA: IconSet = IconSet {
    name: "catppuccin-mocha",
    icon: mocha,
    named: mocha_named,
    all: &catalog::MOCHA,
};

/// Brand colors on a light background.
pub const LATTE: IconSet = IconSet {
    name: "catppuccin-latte",
    icon: latte,
    named: latte_named,
    all: &catalog::LATTE,
};

pub const SET: IconSet = MOCHA;

pub const fn for_background(dark: bool) -> IconSet {
    if dark { MOCHA } else { LATTE }
}

fn mocha(name: IconName) -> &'static Icon {
    &catalog::MOCHA[lucide::index(name)]
}

fn mocha_named(key: &str) -> Option<&'static Icon> {
    catalog::find(&catalog::MOCHA, key, mocha)
}

fn latte(name: IconName) -> &'static Icon {
    &catalog::LATTE[lucide::index(name)]
}

fn latte_named(key: &str) -> Option<&'static Icon> {
    catalog::find(&catalog::LATTE, key, latte)
}
