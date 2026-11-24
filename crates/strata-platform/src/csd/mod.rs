// SPDX-License-Identifier: MIT

//! Client-side decorations shim on Linux (borderless header move, X11 edge resize, minimise,
//! maximised state, and window menu).
//!
//! Rune lacks:
//! - Interactive edge resize and maximised state for borderless windows
//!   ([anomalyco/rune#309](https://github.com/anomalyco/rune/issues/309),
//!   [#130](https://github.com/anomalyco/rune/issues/130)) and diagonal resize cursors
//!   ([anomalyco/rune#145](https://github.com/anomalyco/rune/issues/145)).
//! - Minimising borderless windows
//!   ([anomalyco/rune#310](https://github.com/anomalyco/rune/issues/310)).
//! - Linux window menu and `wm_capabilities`
//!   ([anomalyco/rune#313](https://github.com/anomalyco/rune/issues/313)).

mod edge;
mod frame;
mod request;
mod state;
mod view;
mod x11;

pub use edge::{Bands, Edge};
pub use frame::{Backend, Frame, Policy, window_options};
pub use request::{Gap, Outcome, Request, Route};
pub use state::{Capabilities, Csd, Notice, Report, WindowState};
pub use view::{attach, decorate, header};

#[cfg(test)]
mod tests;
