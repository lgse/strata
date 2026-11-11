//! The 10xer chord panel: the armed chord, its pill and its panel of next keys.

mod state;
mod view;

pub use state::{ChordMap, ChordOption, ChordOutcome, ChordState};
pub use view::{option_key, panel, panel_key, pill, pill_key};
