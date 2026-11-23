// SPDX-License-Identifier: MIT

use std::time::Duration;

// The tests run on it, and the `--qa` mode.
pub(crate) mod chooser_window;
pub(crate) mod floating;
#[cfg(any(test, feature = "qa"))]
pub(crate) mod offscreen;
pub(crate) mod portal;
pub(crate) mod windowed;

pub(crate) const DISPATCH_BUDGET: Duration = Duration::from_millis(8);
