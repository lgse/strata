// SPDX-License-Identifier: MIT

//! The loops that run the interface. Each owns the default GLib context on the interface
//! thread and dispatches it beside Rune's frames, because `Browser` schedules its work there.

use std::time::Duration;

// The tests run on it, and the `--qa` mode.
#[cfg(any(test, feature = "qa"))]
pub(crate) mod offscreen;
pub(crate) mod windowed;

/// How long one pass may keep dispatching GLib before it lets a frame through. A listing that
/// publishes in chunks keeps a source ready until it ends.
const DISPATCH_BUDGET: Duration = Duration::from_millis(8);
