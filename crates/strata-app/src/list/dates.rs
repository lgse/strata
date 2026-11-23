// SPDX-License-Identifier: MIT

use std::time::Duration;

const EVERY: Duration = Duration::from_secs(30);

/// The refresh's clock: when the period started, and how many have passed.
#[derive(Debug, Default)]
pub(crate) struct Dates {
    since: Option<f64>,
    epoch: u64,
}

/// What a poll of the refresh asks: whether the dates read again, and when to wake.
#[derive(Debug, PartialEq)]
pub(crate) struct Tick {
    pub(crate) moved: bool,
    pub(crate) wake: Option<Duration>,
}

impl Dates {
    /// How many periods have passed: in the cells' version while dates are relative.
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn tick(&mut self, live: bool, clock: f64) -> Tick {
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
