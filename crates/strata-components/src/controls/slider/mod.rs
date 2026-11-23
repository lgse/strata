// SPDX-License-Identifier: MIT

mod range;
mod state;
mod view;

pub use range::Range;
pub use state::{SliderChange, SliderState};
pub(crate) use state::{takes_wheel, wheel_notches};
pub use view::{Slider, Snap};

#[cfg(test)]
mod tests;
