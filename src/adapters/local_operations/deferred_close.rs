// SPDX-License-Identifier: MIT

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use super::copy_engine::sleep;

/// How long after `closetimeo` the SMB client may take to close the handle.
const MARGIN: Duration = Duration::from_millis(500);

/// The longest a wait lasts, as `closetimeo` may be set to days.
const LIMIT: Duration = Duration::from_secs(10);

const FIRST_PAUSE: Duration = Duration::from_millis(10);
const LONGEST_PAUSE: Duration = Duration::from_millis(50);

pub(super) enum Pause {
    Retry,
    Spent,
    Cancelled,
}

pub(super) struct Wait {
    left: Duration,
    pause: Duration,
}

impl Wait {
    pub(super) fn after(closetimeo: Duration) -> Self {
        Self {
            left: closetimeo.saturating_add(MARGIN).min(LIMIT),
            pause: FIRST_PAUSE,
        }
    }

    pub(super) fn pause(&mut self, cancelled: &AtomicBool) -> Pause {
        if self.left.is_zero() {
            return Pause::Spent;
        }
        let waited = self.pause.min(self.left);
        sleep(waited);
        self.left -= waited;
        self.pause = (self.pause * 2).min(LONGEST_PAUSE);
        if cancelled.load(Ordering::Acquire) {
            Pause::Cancelled
        } else {
            Pause::Retry
        }
    }
}
