//! The Ctrl+F filter bar: its state, the Enter target's rules and its element.

mod state;
mod target;
mod view;

pub use state::{FilterIntent, FilterState, Polled, Shows};
pub use target::Results;
pub use view::{FilterBar, FilterLabels, position_of, status};
