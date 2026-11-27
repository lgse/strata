// SPDX-License-Identifier: MIT

//! The form's state: the chosen page and a keyboard being carried out of a hidden page.

use std::cell::Cell;
use std::time::Duration;

use rune::Tx;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormPage {
    /// Unique among the form's pages; keys the tab and the panel.
    pub key: String,
    pub title: String,
    /// Draws the error mark on the tab and describes the tab as having errors.
    pub has_errors: bool,
}

impl FormPage {
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            has_errors: false,
        }
    }

    /// Mark the page as holding an invalid field, or clear the mark.
    pub fn errors(mut self, on: bool) -> Self {
        self.has_errors = on;
        self
    }
}

/// Whether a page the user isn't looking at stays built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mounting {
    #[default]
    Kept,
    Rebuilt,
}

/// The form's state, held by the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabbedFormState {
    selected: usize,
    /// Counts page changes: a rebuilt page's body is keyed by it, so each visit mounts afresh.
    pub(super) visits: u64,
    /// Last `visits` value committed by `TabbedForm::build`, so a same-frame follow-up poll does
    /// not overwrite the tab focus before the new panel has mounted.
    pub(super) built_visit: Cell<u64>,
    /// Where a page switch from the keyboard sends a keyboard the hidden page dropped.
    carry: Option<Carry>,
}

/// Focus carry across page switches: enters the newly shown panel once mounted (staying on its
/// tab if the panel holds nothing focusable), or restores tab focus if a remounted `TextEdit`
/// kept `focused = true` across unmount.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Carry {
    Panel { panel: String, ring: bool },
    Tab { key: String, ring: bool },
}

impl TabbedFormState {
    /// The first page chosen.
    pub fn new() -> Self {
        Self::default()
    }

    /// The chosen page's index.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Choose page `i`. The view clamps an index past the last page.
    pub fn select(&mut self, i: usize) {
        if self.selected != i {
            self.selected = i;
            self.visits += 1;
        }
    }

    pub(super) fn carry_panel(&mut self, panel: String, ring: bool, tx: &mut Tx) {
        self.carry = Some(Carry::Panel { panel, ring });
        tx.wake_after(Duration::ZERO);
    }

    pub(super) fn keep_tab(&mut self, key: String, ring: bool, tx: &mut Tx) {
        self.carry = Some(Carry::Tab { key, ring });
        tx.wake_after(Duration::ZERO);
    }

    pub fn show_error(&mut self, i: usize, field: impl Into<String>, tx: &mut Tx) {
        self.carry = None;
        self.select(i);
        tx.focus(field, tx.focus_visible() || tx.by_keys());
    }

    pub fn tick(&mut self, tx: &mut Tx) -> bool {
        if self.carry.is_some() && self.built_visit.get() != self.visits {
            tx.wake_after(Duration::ZERO);
            return false;
        }
        match self.carry.take() {
            None => false,
            Some(Carry::Panel { panel, ring }) => {
                // On the new page's tab now: into the page, if it holds anything focusable.
                tx.focus(panel, ring);
                true
            }
            Some(Carry::Tab { key, ring }) => {
                if tx.focused() == Some(key.as_str()) {
                    return false;
                }
                tx.focus(key, ring);
                true
            }
        }
    }
}
