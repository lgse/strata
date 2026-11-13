// SPDX-License-Identifier: MIT

//! The Modified column's relative dates ("Just now", "5m ago") keep up with the clock, as
//! GTK's do: `src/ui/modified_date.rs` sets every bound date label again every 30 s
//! (`ensure_modified_date_timer`), whatever its age and its format. Here the List view does
//! it only while the saved format is relative, the one whose text moves with the clock, and
//! the folder shown has rows: every 30 s the rows drawn build again ([`Dates::epoch`] is in
//! their cells' version), and a date read then reads the clock then.

use std::time::Duration;

/// GTK's period (`Duration::from_secs(30)` in `ensure_modified_date_timer`).
const EVERY: Duration = Duration::from_secs(30);

/// The refresh's clock: when the period started, and how many have passed.
#[derive(Debug, Default)]
pub(super) struct Dates {
    since: Option<f64>,
    epoch: u64,
}

/// What a poll of the refresh asks: whether the dates read again, and when to wake.
#[derive(Debug, PartialEq)]
pub(super) struct Tick {
    pub(super) moved: bool,
    pub(super) wake: Option<Duration>,
}

impl Dates {
    /// How many periods have passed: in the cells' version while dates are relative.
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// One poll at `clock`, in seconds. `live`: the format is relative and rows show. A
    /// period starts at the first live poll, and ends 30 s on, when the dates read again.
    pub(super) fn tick(&mut self, live: bool, clock: f64) -> Tick {
        if !live {
            self.since = None;
            return Tick {
                moved: false,
                wake: None,
            };
        }
        let since = *self.since.get_or_insert(clock);
        let left = EVERY.as_secs_f64() - (clock - since);
        // A wake lands on its deadline give or take the clock's rounding.
        if left > 1e-3 {
            return Tick {
                moved: false,
                wake: Some(Duration::from_secs_f64(left)),
            };
        }
        self.since = Some(clock);
        self.epoch += 1;
        Tick {
            moved: true,
            wake: Some(EVERY),
        }
    }
}

#[cfg(test)]
mod tests;
