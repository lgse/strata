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

/// A keyboard on the page a switch hides: the switch's commit makes the page inert (or
/// removes it), which drops focus. Only the focused key is known when the switch is asked
/// for, not whether the page holds it, so the next poll checks what was dropped: with nothing
/// focused, the keyboard goes to the first of `targets`, and if that shows nothing focusable,
/// to the next a poll later. A keyboard outside the form keeps its place.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Carry {
    /// The next target first.
    targets: Vec<String>,
    ring: bool,
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
    /// if the switch dropped it (it was on the page now hidden), move it to the first of
    /// `targets` with a focusable element. Replaces a carry not yet done.
    pub(super) fn carry(&mut self, targets: Vec<String>, ring: bool, tx: &mut Tx) {
        self.carry = Some(Carry { targets, ring });
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
        let Some(carry) = self.carry.as_mut() else {
            return false;
        };
        if tx.focused().is_some() || carry.targets.is_empty() {
            self.carry = None;
            return false;
        }
        let target = carry.targets.remove(0);
        tx.focus(target, carry.ring);
        tx.wake_after(Duration::ZERO);
        true
    }
}
