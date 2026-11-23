// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod adapter;
mod clicks;
mod machine;
mod rename;
mod settings;

#[cfg(test)]
mod tests;

pub use adapter::PointerIntent;
pub use machine::{ActivateBy, Button, Outcome, PointerMachine, Press, Release, Spot, Surface};
pub use settings::{ClickCount, ClickModes, PointerSettings};

#[cfg(doc)]
use crate::collection::selection::{Extend, Selection};
