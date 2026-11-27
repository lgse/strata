// SPDX-License-Identifier: MIT

//! Linux platform shims for [Rune](https://github.com/anomalyco/rune) apps.
//!
//! Each module fills one gap in Rune's Linux support, sits behind its own cargo feature, and
//! links the Rune issue that tracks the upstream fix. A module is deprecated for one release
//! once Rune ships its feature, then removed in the next minor release.
//!
//! This crate is generic: it depends on `rune` alone and carries no Strata names in its API.

#[cfg(feature = "clipboard")]
pub mod clipboard;
#[cfg(feature = "dnd")]
pub mod dnd;
#[cfg(feature = "foreign-parent")]
pub mod foreign_parent;
#[cfg(feature = "glib-bridge")]
pub mod glib_bridge;
#[cfg(feature = "glib-loop")]
pub mod glib_loop;
#[cfg(feature = "single-instance")]
pub mod single_instance;
#[cfg(feature = "_transfer")]
#[doc(hidden)]
pub mod transfer;
