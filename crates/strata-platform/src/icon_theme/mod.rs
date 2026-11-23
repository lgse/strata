// SPDX-License-Identifier: MIT

//! Freedesktop icon theme lookup delivered as `rune::Pixels`.
//!
//! Rune lacks user icon-theme resolution (`Inherits`, `index.theme` size/scale matching),
//! lookup by icon/MIME name with fallbacks, and distinguishing missing icons from decode
//! failures ([anomalyco/rune#21](https://github.com/anomalyco/rune/issues/21)). Symbolic icons
//! are not recoloured because Rune cannot tint `Pixels` dynamically.

mod index;
mod loader;
mod matching;
mod request;
mod resolver;
#[cfg(test)]
mod test_support;

pub use loader::{IconLoader, IconState, LoadedIcon, Options, THREAD_NAME};
pub use request::{IconRequest, IconSource, themed_names};
pub use resolver::{FALLBACK_THEME, IconResolver, Resolved, SearchPath};
