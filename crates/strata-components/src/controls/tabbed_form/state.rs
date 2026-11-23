// SPDX-License-Identifier: MIT

//! The form's state: the chosen page and a keyboard being carried out of a hidden page.

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
    /// Where a page switch from the keyboard sends a keyboard the hidden page dropped.
    carry: Option<Carry>,
}

/// Keyboard focus carry across page switches: handles Rune's focus rescue when the old page
/// unmounts or becomes inert (`anomalyco/rune#1298`, `#1302`), and restores tab focus if a
/// remounted `TextEdit` kept `focused = true` across unmount.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Carry {
    Page {
        /// Where focus was when the switch was asked for; `None` once the carry is on the tab.
        from: Option<String>,
        panel: String,
        tab: String,
        tabs_prefix: String,
        ring: bool,
    },
    Tab {
        key: String,
        ring: bool,
    },
}

fn is_form_tab(focused: &str, prefix: &str) -> bool {
    focused
        .strip_prefix(prefix)
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Targets for [`TabbedFormState::carry_page`] when a keyboard page switch hides the old page.
pub(super) struct PageCarry {
    /// The key focused when the switch was asked for.
    pub from: String,
    pub panel: String,
    pub tab: String,
    pub tabs_prefix: String,
    pub ring: bool,
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

    pub(super) fn carry_page(&mut self, page: PageCarry, tx: &mut Tx) {
        self.carry = Some(Carry::Page {
            from: Some(page.from),
            panel: page.panel,
            tab: page.tab,
            tabs_prefix: page.tabs_prefix,
            ring: page.ring,
        });
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
        match self.carry.as_mut() {
            None => false,
            Some(Carry::Page {
                from,
                panel,
                tab,
                tabs_prefix,
                ring,
            }) => {
                let ring = *ring;
                if from.is_none() {
                    // On the new page's tab now: into the page, if it holds anything focusable.
                    let panel = panel.clone();
                    self.carry = None;
                    tx.focus(panel, ring);
                    return true;
                }
                let focused = tx.focused();
                let dropped = focused != from.as_deref()
                    || focused.is_none_or(|key| is_form_tab(key, tabs_prefix));
                if !dropped {
                    self.carry = None;
                    return false;
                }
                *from = None;
                tx.focus(tab.clone(), ring);
                tx.wake_after(Duration::ZERO);
                true
            }
            Some(Carry::Tab { key, ring }) => {
                let (key, ring) = (key.clone(), *ring);
                self.carry = None;
                if tx.focused() == Some(key.as_str()) {
                    return false;
                }
                tx.focus(key, ring);
                true
            }
        }
    }
}
