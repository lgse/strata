// SPDX-License-Identifier: MIT

use strata_kit::look::Look;
use strata_kit::overrides::Overrides;
use strata_kit::shape::LookParams;
use strata_kit::theme::Theme;

use super::shell::{Desktop, OverridesStatus};

pub const SETTLE: f64 = 0.075;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Debounce {
    due: Option<f64>,
}

impl Debounce {
    /// A change at `now`: the read moves to [`SETTLE`] after it. Returns when that is.
    pub fn changed(&mut self, now: f64) -> f64 {
        let due = now + SETTLE;
        self.due = Some(due);
        due
    }

    /// Whether the read is due at `now`; true once per burst.
    pub fn take_due(&mut self, now: f64) -> bool {
        // A frame's clock can land a hair before the sum; a microsecond never splits a burst.
        if self.due.is_some_and(|due| now + 1e-6 >= due) {
            self.due = None;
            return true;
        }
        false
    }

    /// When the pending read is due, if one is.
    pub fn due(&self) -> Option<f64> {
        self.due
    }
}

pub fn params(desktop: &Desktop) -> LookParams {
    LookParams {
        text_scale: desktop.text_scale,
        animations: desktop.animations,
        ..LookParams::default()
    }
}

pub fn build(theme: &Theme, desktop: &Desktop, overrides: &Overrides) -> Look {
    overrides.applied(theme.look().with_params(&params(desktop)))
}

pub fn read(bytes: Option<&[u8]>) -> (Overrides, bool) {
    match bytes {
        Some(bytes) => (Overrides::from_bytes(bytes), true),
        None => (Overrides::default(), false),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The same overrides, and the file there or not as before: nothing to do.
    Nothing,
    Presence,
    /// Other overrides: rebuild the look and move to it, as a theme switch does.
    Look,
}

impl Change {
    pub fn of(current: &Overrides, was_present: bool, next: &Overrides, present: bool) -> Change {
        if next != current {
            Change::Look
        } else if present != was_present {
            Change::Presence
        } else {
            Change::Nothing
        }
    }
}

/// The readout's status after a read.
pub fn status(overrides: &Overrides, present: bool, reloads: u32) -> OverridesStatus {
    OverridesStatus {
        present,
        problems: overrides.errors().len(),
        reloads,
    }
}
