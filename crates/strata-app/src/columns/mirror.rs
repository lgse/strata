// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use strata_fs::app::Browser;

use super::Columns;
use crate::slots::ShownView;

pub(crate) const WAIT: Duration = Duration::from_millis(75);

#[derive(Clone)]
struct Gate {
    /// The saved "Mirror the selection in Columns view" choice.
    enabled: Rc<Cell<bool>>,
    single_click_previews: Rc<Cell<bool>>,
    keyboard: Rc<Cell<bool>>,
    /// An inline rename is open: the child column stays as it is under an edit.
    editing: Rc<Cell<bool>>,
    /// Which view is shown: Columns mirrors only while it is (`mode() != Columns`).
    shown: ShownView,
}

impl Gate {
    fn open(&self, by_removal: bool) -> bool {
        let moved_by = by_removal || self.keyboard.get();
        self.enabled.get() && moved_by && !self.editing.get() && self.shown.is::<Columns>()
    }
}

pub(crate) struct Mirror {
    gate: Gate,
    /// Where the pointer last was, in window points.
    pointer: Cell<Option<(f32, f32)>>,
    waiting: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Mirror {
    /// Whether the saved choice mirrors the selection at all.
    pub(crate) fn enabled(&self) -> bool {
        self.gate.enabled.get()
    }

    pub(crate) fn set_preferences(&self, enabled: bool, single_click_previews: bool) {
        self.gate.enabled.set(enabled);
        self.gate.single_click_previews.set(single_click_previews);
    }

    pub(crate) fn new(enabled: bool, single_click_previews: bool, shown: ShownView) -> Self {
        Self {
            gate: Gate {
                enabled: Rc::new(Cell::new(enabled)),
                single_click_previews: Rc::new(Cell::new(single_click_previews)),
                keyboard: Rc::new(Cell::new(true)),
                editing: Rc::default(),
                shown,
            },
            pointer: Cell::new(None),
            waiting: Rc::default(),
        }
    }

    pub(crate) fn note_input(&self, keyboard: bool) {
        self.gate.keyboard.set(keyboard);
    }

    pub(crate) fn note_pointer(&self, at: (f32, f32)) {
        let moved = self
            .pointer
            .replace(Some(at))
            .is_some_and(|from| from != at);
        if moved {
            self.gate.keyboard.set(false);
        }
    }

    pub(crate) fn hold(&self, editing: bool) {
        self.gate.editing.set(editing);
    }

    pub(crate) fn now(&self, browser: &Rc<Browser>, depth: usize, position: usize) {
        if let Some(earlier) = self.waiting.borrow_mut().take() {
            earlier.remove();
        }
        if self.gate.enabled.get() {
            mirror(browser, depth, position);
        }
    }

    pub(crate) fn follow(
        &self,
        browser: &Rc<Browser>,
        depth: usize,
        position: usize,
        by_removal: bool,
    ) {
        if !self.gate.open(by_removal) {
            return;
        }
        if let Some(earlier) = self.waiting.borrow_mut().take() {
            earlier.remove();
        }
        let browser = Rc::downgrade(browser);
        let gate = self.gate.clone();
        let waiting = self.waiting.clone();
        let source = glib::timeout_add_local_once(WAIT, move || {
            waiting.borrow_mut().take();
            if let Some(browser) = browser.upgrade().filter(|_| gate.open(by_removal)) {
                mirror(&browser, depth, position);
                if gate.single_click_previews.get() {
                    mirror_preview(&browser, depth, position);
                }
            }
        });
        self.waiting.replace(Some(source));
    }
}

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

fn mirror_preview(browser: &Rc<Browser>, depth: usize, position: usize) {
    if browser.pending_navigation_generation().is_some() {
        return;
    }
    let Some((focused_depth, focused_position, entry)) = browser.focused_item() else {
        return;
    };
    if (focused_depth, focused_position) == (depth, position)
        && !entry.is_directory()
        && let Some(target) = crate::preview::preview_target(Some(entry))
    {
        browser.request_automatic_preview(target);
    }
}
