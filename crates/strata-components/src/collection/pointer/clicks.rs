// SPDX-License-Identifier: MIT

use super::machine::Button;
use crate::contract::{EntryId, Point};

/// The last press, for the next one to count on from.
#[derive(Clone, Copy, Debug)]
struct Last {
    button: Button,
    target: Option<EntryId>,
    window: Point,
    at: f64,
    count: u32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Clicks {
    last: Option<Last>,
}

impl Clicks {
    /// Counts a press: 1, or one more than the press before when it continues the run.
    pub(super) fn press(
        &mut self,
        button: Button,
        target: Option<EntryId>,
        window: Point,
        now: f64,
        interval: f64,
        distance: f32,
    ) -> u32 {
        let continues = self.last.filter(|l| {
            l.button == button
                && l.target == target
                && now - l.at < interval
                && (window.x - l.window.x).abs() <= distance
                && (window.y - l.window.y).abs() <= distance
        });
        let count = continues.map_or(1, |l| l.count.saturating_add(1));
        self.last = Some(Last {
            button,
            target,
            window,
            at: now,
            count,
        });
        count
    }

    /// The last press dragged or was cancelled: only clicks count, so it ends the run.
    pub(super) fn break_run(&mut self) {
        self.last = None;
    }
}
