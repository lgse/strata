// SPDX-License-Identifier: MIT

use std::time::{Duration, SystemTime};

use super::{
    copy_engine::{now, sleep},
    fs_ops::{Entry, FsOps},
    leftovers::Deadline,
    mount_profile::known_mount,
};
use crate::adapters::volume::SillyNames;

/// How long after `closetimeo` the SMB client may take to close the handle.
const MARGIN: Duration = Duration::from_millis(500);

const LIMIT: Duration = Duration::from_secs(10);

const FIRST_PAUSE: Duration = Duration::from_millis(10);
const LONGEST_PAUSE: Duration = Duration::from_millis(50);

pub(super) enum Pause {
    Retry,
    Spent,
    Cancelled,
}

pub(super) struct Wait {
    started: SystemTime,
    budget: Duration,
    pause: Duration,
}

impl Wait {
    pub(super) fn after(closetimeo: Duration) -> Self {
        Self::since(now(), closing(closetimeo))
    }

    pub(super) fn since(started: SystemTime, budget: Duration) -> Self {
        Self {
            started,
            budget: budget.min(LIMIT),
            pause: FIRST_PAUSE,
        }
    }

    pub(super) fn pause(&mut self, stopped: &dyn Fn() -> bool) -> Pause {
        let Ok(spent) = now().duration_since(self.started) else {
            return Pause::Spent;
        };
        let left = self.budget.saturating_sub(spent);
        if left.is_zero() {
            return Pause::Spent;
        }
        sleep(self.pause.min(left));
        self.pause = (self.pause * 2).min(LONGEST_PAUSE);
        if stopped() {
            Pause::Cancelled
        } else {
            Pause::Retry
        }
    }
}

pub(super) fn closing(closetimeo: Duration) -> Duration {
    closetimeo.saturating_add(MARGIN).min(LIMIT)
}

pub(super) struct WaitCancelled;

pub(super) fn retry_folder_rename<T, E>(
    fs: &dyn FsOps,
    folder: Entry<'_>,
    deadline: &Deadline,
    cancelled: &dyn Fn() -> bool,
    refused: impl Fn(&E) -> bool,
    mut rename: impl FnMut() -> Result<T, E>,
) -> Result<Result<T, E>, WaitCancelled> {
    let mut error = match rename() {
        Err(error) if refused(&error) => error,
        done => return Ok(done),
    };
    let (_, capabilities) = known_mount(fs, folder.dir);
    let Some(closetimeo) = capabilities
        .deferred_close
        .filter(|_| capabilities.silly_names != Some(SillyNames::Unlisted))
    else {
        return Ok(Err(error));
    };
    if !fs.stat(folder, false).is_ok_and(|stat| stat.is_directory()) {
        return Ok(Err(error));
    }
    let mut wait = deadline.wait(closing(closetimeo));
    loop {
        match wait.pause(cancelled) {
            Pause::Retry => {}
            Pause::Spent => return Ok(Err(error)),
            Pause::Cancelled => return Err(WaitCancelled),
        }
        error = match rename() {
            Err(error) if refused(&error) => error,
            done => return Ok(done),
        };
    }
}
