// SPDX-License-Identifier: MIT

//! View switching and the look: package A8 of the parity plan
//! (`docs/rune/parity/packages/A8.md`).
//!
//! - **The view switch** ([`views`]): Ctrl+1, Ctrl+2 and Ctrl+3 show Columns, Icons and List
//!   while the window is open, and the choice is saved as `browser_mode`.
//!
//! Nothing of it is drawn yet, and it is no part of the window yet: the view switch's
//! commands are the window's own (`commands::own`), and the part comes with the Appearance
//! popover and its state.

pub(crate) mod views;

#[cfg(test)]
mod tests;
