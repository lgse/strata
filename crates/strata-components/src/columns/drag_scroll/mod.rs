//! Column autoscroll during a file drag (`operations/drag-and-drop/column-autoscroll`), and the
//! column a held drag would drop into.
//!
//! Holding an accepted drag within 44 points of the strip's left or right edge scrolls the
//! strip that way, and within 44 points of the top or bottom of the listing under it scrolls
//! that listing. The speed peaks within 16 points of the edge, at 720 points a second sideways
//! and 520 down, starting at 35 % and reaching it after 350 ms in the same direction
//! ([`speed`]). Each frame retargets a Rune glide a lead ahead, as C1d's and C24's autoscrolls
//! do, rather than stepping instant scrolls; it stops dead where the view is when the drag
//! leaves the edge or the strip, drops or ends, or the view can't go further.
//!
//! The drag's position arrives as data ([`DragScroll::over`]), so any binder can feed it:
//! Rune's, through the columns' drop zones and the strip's blank space (C1e's
//! `Collection::on_drag_over`), or X2's system shim, which sends no moves of its own, through
//! [`ColumnsState::drag_over`](super::ColumnsState::drag_over). The column under the drag is
//! worked out each frame from the drag's last point and the strip as it stands, since content
//! scrolls under a parked pointer (Strata tracks it the same way: GTK doesn't re-evaluate drop
//! targets then).

mod machine;
pub mod speed;

#[cfg(test)]
mod tests;

pub use machine::{ColumnGeometry, DragScroll, Scroll, Step, StripGeometry};
pub(crate) use machine::{GLIDE, column_at};
