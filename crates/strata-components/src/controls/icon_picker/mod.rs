// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod data;
mod emoji;
mod grid;
mod matching;
mod recent;
mod state;
mod tabs;
mod view;
mod words;

#[cfg(test)]
mod tests;

pub use data::{Choice, PickerCategory, PickerItem, PickerSource};
pub use emoji::DEFAULT_VERSION;
pub(crate) use grid::drawn_emoji;
pub use grid::drawn_text;
pub use recent::{RECENT_MAX, remember};
pub use state::{IconPickerState, Page};
pub use view::IconPicker;
pub use words::PickerWords;
