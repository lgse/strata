// SPDX-License-Identifier: MIT

mod machine;
/// Edge-zone autoscroll speed calculation.
pub mod speed;

#[cfg(test)]
mod tests;

pub use machine::{ColumnGeometry, DragScroll, Scroll, Step, StripGeometry};
pub(crate) use machine::{GLIDE, column_at};
