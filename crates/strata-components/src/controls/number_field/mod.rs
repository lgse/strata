// SPDX-License-Identifier: MIT

mod state;
mod view;

pub use crate::controls::slider::Range;
pub use state::NumberFieldState;
pub use view::NumberField;

#[cfg(test)]
mod tests;
