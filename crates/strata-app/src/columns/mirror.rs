// SPDX-License-Identifier: MIT

//! Mirroring: the child column that follows the keyboard cursor. Moving onto a folder shows
//! its contents as the next column without moving focus; moving onto a file closes that
//! column. Neither `Browser` nor the Columns component does this: it is view policy.
//!
//! Ported from the GTK views, `src/ui/browser/events/selection.rs` (`mirror_focused_folder`,
//! `child_mirror_blocked`, `apply_child_mirror`, `mirror_entry`). A fix there has to be
//! carried here until the GTK views are deleted. Left out, with what brings each back:
//!
//! - a rename or a new entry being edited blocks the mirror: V9;
//! - a 10xer visual range, a filtered column and the automatic preview of a file: interface
//!   the slice leaves out.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use strata_fs::app::Browser;

/// How long the cursor rests before its folder opens: `FOCUS_PREVIEW_DELAY` in
/// `src/ui/preview.rs`. Holding an arrow key so opens nothing on the way.
pub(crate) const WAIT: Duration = Duration::from_millis(75);

pub(crate) struct Mirror {
    /// The saved "Mirror the selection in Columns view" choice.
    enabled: bool,
    /// Whether the last input that moved the cursor was a key. The GTK views start that way
    /// too (`NavigationInput::default`).
    keyboard: Rc<Cell<bool>>,
    /// Where the pointer last was, in window points.
    pointer: Cell<Option<(f32, f32)>>,
    waiting: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Mirror {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            keyboard: Rc::new(Cell::new(true)),
            pointer: Cell::new(None),
            waiting: Rc::default(),
        }
    }

    pub(crate) fn note_input(&self, keyboard: bool) {
        self.keyboard.set(keyboard);
    }

    /// The pointer is at `at`. Only a move from one place to another ends keyboard
    /// navigation (`InputOwnership::pointer_motion` in `src/ui/input_ownership.rs`): the
    /// toolkit reports a parked pointer whenever what is under it changes, as a key's own
    /// redraw does, and the first report only says where it is parked.
    pub(crate) fn note_pointer(&self, at: (f32, f32)) {
        let moved = self
            .pointer
            .replace(Some(at))
            .is_some_and(|from| from != at);
        if moved {
            self.keyboard.set(false);
        }
    }

    /// The cursor came to rest on `position` in column `depth`. After [`WAIT`], if it is
    /// still there, the child column follows it. Only the keyboard mirrors, unless a
    /// deletion moved the cursor (`by_removal`).
    pub(crate) fn follow(
        &self,
        browser: &Rc<Browser>,
        depth: usize,
        position: usize,
        by_removal: bool,
    ) {
        if !self.enabled || !(by_removal || self.keyboard.get()) {
            return;
        }
        if let Some(earlier) = self.waiting.borrow_mut().take() {
            earlier.remove();
        }
        let browser = Rc::downgrade(browser);
        let keyboard = self.keyboard.clone();
        let waiting = self.waiting.clone();
        // On GLib's clock, as the GTK views wait: the browser needs no frame to open a
        // column, and its events reach the interface through the queue like any other.
        let source = glib::timeout_add_local_once(WAIT, move || {
            waiting.borrow_mut().take();
            if let Some(browser) = browser.upgrade()
                && (by_removal || keyboard.get())
            {
                mirror(&browser, depth, position);
            }
        });
        self.waiting.replace(Some(source));
    }
}

/// Opens the folder under the cursor as the next column, or closes that column for a file.
/// Nothing happens once the cursor has moved on, or while the browser is still checking a
/// folder it was asked to open, which takes the focus itself.
fn mirror(browser: &Rc<Browser>, depth: usize, position: usize) {
    if browser.pending_navigation_generation().is_some() {
        return;
    }
    let Some((focused_depth, focused_position, entry)) = browser.focused_item() else {
        return;
    };
    if (focused_depth, focused_position) != (depth, position) {
        return;
    }
    if entry.is_directory() {
        browser.show_child(depth, entry.location);
    } else {
        browser.close_column(depth + 1);
    }
}
