//! The Catppuccin catalogs: Strata's Brand colors. The same icons as [`super::lucide`], but
//! each Catppuccin file-type icon draws in its own colours, in the Mocha or Latte flavour; Rune
//! draws a body that names its colours in them, so the element's colour doesn't reach it. The
//! Lucide icons still take the element's colour. Licence: MIT (Catppuccin VSCode Icons),
//! `crates/strata-kit/icons/README.md`.
//!
//! Work package: K3, `docs/work/phase-1-leaves/K3-icon-catalogs.md`.
//!
//! # Choosing a catalog
//!
//! Which catalog a look carries is the host's choice, from Strata's File icon colors setting:
//!
//! ```ignore
//! look.icons = match file_icon_colors {
//!     FileIconColors::Brand => catppuccin::for_background(theme.dark_background()),
//!     FileIconColors::Theme => lucide::SET,
//! };
//! ```
//!
//! Do it again after every theme switch: `Look::from_kit`, and so `Theme::look`, starts from
//! [`super::lucide::SET`].

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

/// The Catppuccin catalog in Strata's default flavour, Mocha, which Strata also uses when a
/// theme's background can't be read.
pub const SET: IconSet = MOCHA;

/// Strata's flavour for a background: Mocha when `dark`, else Latte. Pass
/// `Theme::dark_background()`, Strata's own rule, rather than `kit.dark`.
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
