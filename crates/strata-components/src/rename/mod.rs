// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod editor;
mod intent;
/// Stable element key builders for the inline rename editor.
pub mod keys;
mod state;
mod stem;

#[cfg(test)]
mod tests;

pub use editor::RenameEditor;
pub use intent::{By, Kept, Polled, RenameIntent};
pub use state::RenameState;
pub use stem::{Initial, initial_selection};

#[cfg(doc)]
use crate::collection::element::CollectionState;
