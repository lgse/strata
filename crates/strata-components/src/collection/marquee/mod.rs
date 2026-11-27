// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod adapter;
mod machine;
mod settings;
mod speed;
/// Marquee selection overlay view helpers.
pub mod view;

#[cfg(test)]
mod tests;

pub use adapter::Marquee;
pub use machine::{MarqueeMachine, Outcome, Surface, mode_for};
pub use settings::MarqueeSettings;
pub use speed::{EdgeScroll, edge_speed, glide_lead, stop_lead};

#[cfg(doc)]
use crate::{collection::selection::Selection, contract::CollectionGeometry};
