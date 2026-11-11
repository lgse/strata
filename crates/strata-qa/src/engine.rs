// SPDX-License-Identifier: MIT

//! What the QA mode decides without knowing the toolkit: which nodes a snapshot reports, in
//! what order, and what each of them is.

mod fields;
mod locate;
mod lookup;
mod scan;
mod scope;
mod wanted;

use std::time::Duration;

pub use self::{
    fields::{editable_fields, editable_texts},
    locate::{Kept, Located, focused, locate, presented_target, screen_rect},
    lookup::{descendants, find_presented, presented, rendered, result_rows, windows_titled},
    scan::{PaneNodes, Scene, scan},
    scope::{Memory, Remembered, Scope},
    wanted::{Wanted, has_state},
};

/// How long a step looks for its target before it gives up.
pub const LOCATE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a waiting step lets the application run between two looks.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);
