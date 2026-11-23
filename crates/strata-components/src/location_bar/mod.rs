// SPDX-License-Identifier: MIT

mod build;
mod completion;
mod crumbs;
mod data;
mod drops;
mod entry;
mod intent;
mod menu;
mod popup;
mod state;

#[cfg(test)]
mod tests;

pub use build::{Access, LocationBar, crumb_key, field_key, option_key};
pub use data::{
    CRUMB_CHARS, Candidate, CompletionQuery, Completions, Crumb, LocationLabels, Trail,
    elide_middle,
};
pub use intent::{LocationIntent, Via};
pub use state::{COPIED_FOR, LocationBarState, Polled, SPRING_DELAY};
