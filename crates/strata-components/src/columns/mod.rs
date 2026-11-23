// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

mod badge;
mod column;
/// Edge-zone autoscroll during file drags.
pub mod drag_scroll;
mod fit;
mod intent;
/// Stable element key builders for columns view.
pub mod keys;
mod pointer;
mod reveal;
mod sources;
mod state;
mod strip;
mod view;
/// Column width calculation and autofit helpers.
pub mod width;

#[cfg(test)]
mod tests;

pub use intent::ColumnsIntent;
pub use sources::ColumnSources;
pub use state::{ColumnState, ColumnsState, Focus, Opening};
pub use view::{ColumnsLabels, ColumnsParts, ColumnsPartsMut, ColumnsView, HeaderCx};
