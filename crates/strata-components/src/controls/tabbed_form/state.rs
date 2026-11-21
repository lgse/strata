// SPDX-License-Identifier: MIT

//! The form's state: the chosen page and a keyboard being carried out of a hidden page.

use std::time::Duration;

use rune::Tx;

/// A page of the form, as data: its key, its tab's title and whether it holds a field the
/// host found invalid.
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
    /// Every page is built every frame; the hidden ones are invisible, inert and out of the
    /// accessibility tree. Each keeps what Rune holds per element across a switch: its scroll
    /// offset, the focus memory of its groups, measured virtual-row heights. The form is as
    /// tall as its tallest page, as GtkNotebook is.
    #[default]
    Kept,
    /// Only the chosen page is built; a switch starts the page afresh (scrolled to its top).
    /// The form is as tall as the chosen page.
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

/// A keyboard carried across a page switch:
///
/// - [`Carry::Page`]: Ctrl+Page Up or Down ran with focus off the tabs, on `from`. Making the old
///   page inert or unmounting it (`Mounting::Rebuilt`) takes focus off `from`: to `None`, or,
///   since Rune `7a707454`, to the next Tab stop around it (its focus rescue,
///   anomalyco/rune#1298 and #1302), which can be outside the form. Only the focused key was
///   known when the switch was asked for, so the next poll checks whether focus is now `None`,
///   on one of the form's tabs or anywhere but `from`: if so, focus was inside the hidden page
///   and moves to `tab`, then a poll later into `panel`, where it stays on `tab` if `panel`
///   holds nothing focusable, as it went through the tab strip before the rescue.
///   Focus outside the form (such as a dialog footer button) stays where it was.
/// - [`Carry::Tab`]: a switch with focus on a tab. Under `Mounting::Rebuilt`, remounting a page
///   whose `TextEdit` kept `focused = true` across an earlier unmount steals focus in the switch's
///   commit; the next poll restores `key` when that happened.
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

    /// After a switch from the keyboard (Ctrl+Page Up or Down) with the keyboard off the tabs:
    /// if the switch dropped focus out of the hidden page, move it to `panel` when it holds a
    /// focusable element, else to `tab`. Replaces a carry not yet done.
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

    /// Keeps focus on tab `key` across a page switch when a remounted `TextEdit` on the shown
    /// page still carried `focused = true` from before it unmounted.
    pub(super) fn keep_tab(&mut self, key: String, ring: bool, tx: &mut Tx) {
        self.carry = Some(Carry::Tab { key, ring });
        tx.wake_after(Duration::ZERO);
    }

    /// After a failed save (Strata's `invalid_field`): choose page `i` and move the keyboard to
    /// the element keyed `field` on it, in the same change, so a keyboard inside the page being
    /// hidden (Return in one of its fields pressed Save) still moves. The ring shows when the
    /// keyboard has been driving. The host also sets the page's [`FormPage::has_errors`] and
    /// keeps every edit: the form holds none of them.
    ///
    /// `field` must key a focusable element on page `i`: if it keys nothing shown there (a
    /// host bug), the page still changes and the keyboard, if it was on the page being hidden,
    /// is dropped.
    pub fn show_error(&mut self, i: usize, field: impl Into<String>, tx: &mut Tx) {
        self.carry = None;
        self.select(i);
        tx.focus(field, tx.focus_visible() || tx.by_keys());
    }

    /// From the host's poll, every frame: carries a keyboard a page switch dropped. True when
    /// it asked to move focus.
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
