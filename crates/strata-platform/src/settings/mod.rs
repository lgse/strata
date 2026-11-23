// SPDX-License-Identifier: MIT

//! XDG Settings portal watcher delivering desktop settings changes onto Rune's loop.
//!
//! Rune reads only `color-scheme` from the Settings portal (not text scale, double-click
//! interval, drag threshold, caret blink, `reduced-motion`, or `remember-recent-files`) and
//! hardcodes a 0.5 s Linux double-click interval in windowed apps.

mod fallback;
mod keys;
mod parse;
mod snapshot;
mod watcher;

#[cfg(test)]
mod tests;

use std::io;

use rune::AppHandle;

pub use fallback::Fallback;
pub use keys::{Key, Setting};
pub use snapshot::DesktopSettings;
pub use watcher::{Event, Options, THREAD_NAME, Watcher};

pub fn follow<S: 'static>(
    handle: &AppHandle<S>,
    at: fn(&mut S) -> &mut DesktopSettings,
) -> io::Result<Watcher> {
    let handle = handle.clone();
    Watcher::start(Options::default(), move |event| {
        let settings = match event {
            Event::Snapshot(settings) | Event::Changed { settings, .. } => settings,
            Event::Fallback(_) => return,
        };
        handle.update_if(move |state, _| {
            let slot = at(state);
            let changed = *slot != settings;
            *slot = settings;
            changed
        });
    })
}
