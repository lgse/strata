//! Strata's look on top of `rune_kit::Kit`: theme mapping, icon catalogs, density and text
//! size. Components read these tokens, never a theme by name.
//!
//! Every module is declared here once (W0). A work package edits only its own module files,
//! so parallel branches never touch this file.

pub mod density;
pub mod fonts;
pub mod icons;
pub mod look;
pub mod overrides;
pub mod shape;
pub mod slots;
pub mod theme;
pub mod tokens;
pub mod typography;

pub use look::fallback;
