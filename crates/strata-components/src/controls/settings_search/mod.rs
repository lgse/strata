// SPDX-License-Identifier: MIT

#[doc(hidden)]
pub mod fixture;
mod index;
pub mod matching;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use index::{IndexPage, IndexRow, SearchIndex};
pub use state::{FLASH_FOR, FLASH_HOLD, SettingsSearchState};
pub use view::{FLASH_ALPHA, Navigate, SearchLabels, SettingsSearch};
