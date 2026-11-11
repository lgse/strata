//! The host's half of K4's live reload and P5's look parameters, shared by the gallery page and
//! `examples/shell`: when a change to the overrides file is read, and how a look is rebuilt.
//!
//! strata-kit does no I/O (K4): the host watches the file, reads it, and hands the bytes in.
//! This module holds what doesn't touch the file: the settle delay and the rebuild.

use strata_kit::look::Look;
use strata_kit::overrides::Overrides;
use strata_kit::shape::LookParams;
use strata_kit::theme::Theme;

use super::shell::{Desktop, OverridesStatus};

/// How long the file must stay unchanged before it is read: Strata's user stylesheet waited
/// 75 ms after the last change (`settings/themes`, "Changes to the file less than 75 ms apart
/// trigger one reload, 75 ms after the last").
pub const SETTLE: f64 = 0.075;

/// Collapses a burst of changes into one read, [`SETTLE`] after the last, on any clock in
/// seconds (Rune's poll clock: virtual in tests).
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

/// The look parameters with the desktop's text scale and animations switch (P5), on Strata's
/// defaults for everything Strata's own settings would set.
pub fn params(desktop: &Desktop) -> LookParams {
    LookParams {
        text_scale: desktop.text_scale,
        animations: desktop.animations,
        ..LookParams::default()
    }
}

/// The look for a theme, the desktop's settings and the user's overrides, in K4's order: the
/// theme, then the parameters, then the overrides.
pub fn build(theme: &Theme, desktop: &Desktop, overrides: &Overrides) -> Look {
    overrides.applied(theme.look().with_params(&params(desktop)))
}

/// What a read of the file says: `None` when there is no file or it can't be read, which
/// restores the theme's look.
pub fn read(bytes: Option<&[u8]>) -> (Overrides, bool) {
    match bytes {
        Some(bytes) => (Overrides::from_bytes(bytes), true),
        None => (Overrides::default(), false),
    }
}

/// What a read of the file changes, against what the host has: K4's host rule is that a read
/// that finds the same overrides changes nothing (`strata_kit::overrides`, "The host").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The same overrides, and the file there or not as before: nothing to do.
    Nothing,
    /// The same overrides (none), but the file appeared or went: only the readout changes, as
    /// an empty file and no file apply alike.
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
