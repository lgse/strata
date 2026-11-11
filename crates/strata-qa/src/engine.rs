// SPDX-License-Identifier: MIT

//! What the QA mode decides without knowing the toolkit: which nodes a snapshot reports, in
//! what order, and what each of them is.

mod activate;
mod check;
mod dispatch;
mod fields;
mod keyboard;
mod locate;
mod lookup;
mod note;
mod observe;
mod pointer;
mod scan;
mod scope;
mod serve;
mod waits;
mod wanted;

use std::time::Duration;

pub use self::{
    check::{Verdict, mismatch},
    dispatch::execute,
    fields::editable_fields,
    locate::{Kept, Located, focused, locate, presented_target, screen_rect},
    lookup::{descendants, find_presented, presented, rendered, result_rows, windows_titled},
    observe::{VisibleRow, off_ui_thread, preferences},
    scan::{PaneNodes, Scene, scan},
    scope::{Memory, Remembered, Scope},
    serve::{command_lines, reply_stream, serve},
    wanted::{Wanted, has_state},
};

/// How long a step looks for its target before it gives up.
pub const LOCATE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a waiting step lets the application run between two looks.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// A layout change from the press makes GTK synthesize motion a frame later, and a
/// release it has not read yet would cancel the click.
const BUTTON_HOLD: Duration = Duration::from_millis(100);
/// Keeps two clicks inside GTK's 400 ms double-click time; nothing may settle between them.
const DOUBLE_CLICK_GAP: Duration = Duration::from_millis(90);
/// Outside the double-click time, so a drag after a selecting click is no double-click.
const SEPARATE_CLICK_GAP: Duration = Duration::from_millis(600);
/// What the application is given to take in one key or button event.
pub const EVENT_GAP: Duration = Duration::from_millis(20);
const POINTER_GAP: Duration = Duration::from_millis(50);
/// A click can land before GTK maps the row for input, so it clicks again after this long.
const SELECT_RETRY_AFTER: Duration = Duration::from_secs(4);
const SETTLE_SAMPLES: usize = 3;
const MENU_RETRY_AFTER: Duration = Duration::from_secs(1);
const KEYBOARD_SELECT_STEPS: usize = 40;
/// How often a step turns the wheel to bring one target into view.
pub const REVEAL_SCROLL_ATTEMPTS: usize = 12;
const ARRIVAL_ATTEMPTS: usize = 5;
/// Approach a point in two steps so GTK sees enter and motion, not a teleport.
const APPROACH_OFFSET: i16 = 6;
