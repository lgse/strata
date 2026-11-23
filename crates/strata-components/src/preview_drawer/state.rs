// SPDX-License-Identifier: MIT

use rune::{Anim, Curve, Easing, TextEdit, Tx, Velocity};
use strata_kit::look::Look;

use super::layout::DrawerRules;

/// How long the drawer slides open or closed (Strata's `TRANSITION`).
pub const SLIDE_SECONDS: f32 = 0.26;

/// The longest find query, in characters.
pub const FIND_MAX_CHARS: usize = 256;

#[derive(Clone, Default)]
pub struct DrawerState {
    open: bool,
    reserved: bool,
    manual: Option<f32>,
    pub(super) dragging: bool,
    /// The manual width a pointer drag started from, put back if the drag is cancelled.
    pub(super) drag_from: Option<f32>,
    pub(super) find_end_pending: bool,
    pub(super) marks: u32,
    pub(super) hint: bool,
    /// The host lights the divider's line, as hover does (`DrawerState::light_line`).
    pub(super) lit: bool,
    pub(super) suspended: bool,
    pub(super) focus_content: u32,
    pub(super) focus_files: u32,
    pub(super) focus_divider: u32,
    pub(super) find: FindBar,
}

impl std::fmt::Debug for DrawerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawerState")
            .field("open", &self.open)
            .field("reserved", &self.reserved)
            .field("manual", &self.manual)
            .field("dragging", &self.dragging)
            .field("suspended", &self.suspended)
            .field("finding", &self.find.open)
            .field("find_query", &self.find.query.text)
            .finish_non_exhaustive()
    }
}

/// The find bar's state. The query outlives the bar: reopening find selects it again.
#[derive(Clone, Default)]
pub struct FindBar {
    pub(super) open: bool,
    pub(super) query: TextEdit,
}

impl DrawerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, look: &Look, tx: &mut Tx) {
        self.open_with(look.animations, tx);
    }

    pub fn open_at_once(&mut self) {
        self.open = true;
        self.marks = self.marks.wrapping_add(1);
    }

    pub(super) fn open_with(&mut self, animations: bool, tx: &mut Tx) {
        if !self.open {
            self.slide(animations, tx);
            self.marks = self.marks.wrapping_add(1);
        }
        self.open = true;
    }

    pub fn close(&mut self, look: &Look, tx: &mut Tx) {
        self.close_with(look.animations, tx);
    }

    pub(super) fn close_with(&mut self, animations: bool, tx: &mut Tx) {
        if self.open {
            self.slide(animations, tx);
        }
        self.open = false;
        self.dragging = false;
        self.hint = false;
        self.reset_find();
    }

    fn slide(&self, animations: bool, tx: &mut Tx) {
        if self.reserved || !animations {
            tx.instant();
            return;
        }
        let mut anim: Anim = Curve::tween(SLIDE_SECONDS, Easing::EASE_IN_OUT).into();
        anim.velocity = Velocity::Zero;
        tx.motion_curve(anim);
    }

    pub fn light_line(&mut self, on: bool) {
        self.lit = on;
    }

    /// Whether the host lights the divider's line ([`DrawerState::light_line`]).
    pub fn line_lit(&self) -> bool {
        self.lit
    }

    /// Whether Columns keeps the slot while the preview is closed.
    pub fn is_reserved(&self) -> bool {
        self.reserved
    }

    pub fn reserve(&mut self, on: bool) {
        self.reserved = on;
    }

    pub fn manual_width(&self) -> Option<f32> {
        self.manual
    }

    /// Sets the manual width, kept at or above the rules' least manual width.
    pub fn set_manual_width(&mut self, rules: &DrawerRules, width: f32) {
        if width.is_finite() {
            self.manual = Some(width.round().max(rules.manual_min));
        }
    }

    /// Forgets the manual width, back to the automatic one.
    pub fn clear_manual_width(&mut self) {
        self.manual = None;
    }

    pub fn is_suspended(&self) -> bool {
        self.open && self.suspended
    }

    pub fn focus_content(&mut self) {
        self.focus_content = self.focus_content.wrapping_add(1);
    }

    pub fn focus_content_when_shown(&self, tx: &mut Tx) {
        tx.focus_when_shown(super::BODY_KEY, true);
    }

    pub fn focus_files(&mut self) {
        self.focus_files = self.focus_files.wrapping_add(1);
    }

    /// Moves keyboard focus to the divider, where the arrow keys move it (Strata's F8).
    pub fn focus_divider(&mut self) {
        self.focus_divider = self.focus_divider.wrapping_add(1);
    }

    /// Whether the find bar is open.
    pub fn is_finding(&self) -> bool {
        self.find.open
    }

    /// Whether the find bar's text field holds keyboard focus.
    pub fn is_find_focused(&self) -> bool {
        self.find.open && self.find.query.focused
    }

    /// Whether the divider is being dragged by the pointer.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn find_query(&self) -> &str {
        &self.find.query.text
    }

    pub fn reset_find(&mut self) {
        if self.find.open {
            self.find_end_pending = true;
        }
        self.find.open = false;
        self.find.query = TextEdit::default();
    }

    pub fn demand(&self, rules: &DrawerRules) -> f32 {
        self.manual.unwrap_or(0.0).max(rules.hide_below)
    }

    pub fn start_find(&mut self, query: &str) {
        self.find.query = TextEdit::new(query);
        self.open_find();
    }

    /// Puts back the manual width a cancelled pointer drag started from.
    pub(super) fn cancel_drag(&mut self) {
        self.manual = self.drag_from.take();
        self.dragging = false;
    }

    /// Opens the find bar with the query selected and the field focused.
    pub(super) fn open_find(&mut self) {
        // Find starts again: an end the view hasn't heard would land after the new search.
        self.find_end_pending = false;
        self.find.open = true;
        let len = self.find.query.text.len();
        self.find.query.set_selection(0, len);
        self.find.query.focused = true;
    }
}
