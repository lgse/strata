// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod adapter;
mod command;
mod keymap;
mod machine;
mod settings;
mod type_ahead;

#[cfg(test)]
mod tests;

pub use adapter::{Keyboard, NothingToSelect, Rested};
pub use command::{Command, RangeKind, Target, Vertical};
pub use keymap::{Binding, KeyMap};
pub use machine::{KeyMachine, Outcome, Surface};
pub use settings::{KeyHandle, KeySettings, Letters};

#[cfg(doc)]
use crate::{collection::selection::Selection, contract::CollectionGeometry};
