//! The Lucide catalog: Strata's Theme colors. Every icon is single-colour and takes the
//! element's colour (a component tints it with Kit's icon roles, the accent by default),
//! including Catppuccin's file-type icons, which also get Strata's 15 % fill. Licences: ISC
//! (Lucide, with Feather's MIT) and MIT (Catppuccin), `crates/strata-kit/icons/README.md`.
//!
//! Work package: K3, `docs/work/phase-1-leaves/K3-icon-catalogs.md`.
//!
//! All three catalogs hold the same icons under the same names: Strata's Lucide interface and
//! generic file-type icons (`strata-folder`), its Catppuccin icons for code and config files
//! (`strata-ctp-rust`), and the few this repository adds (`strata-kit-ellipsis`). They differ
//! only in how a Catppuccin icon is coloured; [`super::catppuccin`] has the brand colours.
//! Every [`IconName`] maps to the same Lucide icon in each ([`semantic`]).
//!
//! `build.rs` generates the icons from `crates/strata-kit/icons/` ([`catalog`]), by the rules
//! in `lucide/generate.rs`.

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
