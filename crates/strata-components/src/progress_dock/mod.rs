// SPDX-License-Identifier: MIT

mod countdown;
mod dialog;
pub mod format;
pub mod jobs;
mod model;
mod rate;
mod view;
mod words;

#[cfg(test)]
mod tests;

pub use countdown::{COMPLETE_FOR, Countdown, FADE};
pub use dialog::{EscapeDoes, ProgressDialog, RATE_WIDTH};
pub use model::{
    COMMIT_EVERY, CardLive, Dock, DockIntent, DockNotice, EndKind, EndShown, IMMEDIATE_ITEMS,
    NOTICE_FOR, Notice, Op, OpId, OpSpec, Placement, Progress, REVEAL_DELAY, STALL_AFTER, Sample,
    Shown, Transfer, reveal_after,
};
pub use rate::{RateMeter, SAMPLE_EVERY, SMOOTHING};
pub use view::{
    BAR_RADIUS, CARD_BAR, CARD_MAX_WIDTH, CARD_MIN_WIDTH, CARD_RADIUS, DIALOG_BAR, DOCK_BOTTOM,
    DOCK_MAX_HEIGHT, DockView,
};
pub use words::{Phrase, Phrase2, Words};
