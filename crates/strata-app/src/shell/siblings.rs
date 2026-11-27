// SPDX-License-Identifier: MIT

use std::sync::atomic::{AtomicU64, Ordering};

use crate::{App, settings::Saved, slots::Later};

/// A window id no other window of the process has.
pub(super) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Which of the process's windows `app` is.
pub(crate) fn window_id(app: &App) -> u64 {
    app.window.id
}

pub(crate) fn window_later(app: &App) -> Later<App> {
    app.window.events.later()
}

/// The same settings for another window of the process ([`Saved::share`]).
pub(crate) fn share_settings(app: &App) -> Saved {
    app.window.settings.share()
}
