// SPDX-License-Identifier: MIT

//! The window as one of the process's: which it is, its queue for a process-wide service, and
//! the settings a window opened beside it shares (`crate::application`).

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

/// The way into the window's queue, for a service of the window's own: what it queues runs in
/// the next `poll`, whichever tab is shown.
pub(crate) fn window_later(app: &App) -> Later<App> {
    app.window.events.later()
}

/// The same settings for another window of the process ([`Saved::share`]).
pub(crate) fn share_settings(app: &App) -> Saved {
    app.window.settings.share()
}
