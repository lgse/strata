//! The drawer's state: a field of each window's state, so every browser window and the chooser
//! has its own. Nothing here is saved; a manual width lives as long as the window's state does.

use rune::{Anim, Curve, Easing, TextEdit, Tx, Velocity};
use strata_kit::look::Look;

use super::layout::DrawerRules;

/// How long the drawer slides open or closed (Strata's `TRANSITION`).
pub const SLIDE_SECONDS: f32 = 0.26;

/// The longest find query, in characters.
pub const FIND_MAX_CHARS: usize = 256;

/// One window's drawer: open or closed, the Columns reservation, the manual width, the find
/// bar, and the pending focus moves.
///
/// Whether the user dismissed the preview on purpose, and so whether moving the cursor onto
/// a file opens it again, is Strata's policy, not this state's: Strata keeps its own
/// "dismissed" flag and calls [`DrawerState::open`] when its rules say so.
#[derive(Clone, Default)]
pub struct DrawerState {
    open: bool,
    reserved: bool,
    manual: Option<f32>,
    pub(super) dragging: bool,
    /// The manual width a pointer drag started from, put back if the drag is cancelled.
    pub(super) drag_from: Option<f32>,
    /// Find ended (the drawer closed, or another file is previewed) and the hosted view hasn't
    /// heard yet: the next build tells it.
    pub(super) find_end_pending: bool,
    /// Counts the drawer's state markers (see `mod.rs`): part of each marker's key, so a marker
    /// that is needed again after it ran mounts afresh and runs again.
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

    /// Shows the preview. It slides open unless the Columns slot is reserved (a reserved
    /// slot never slides) or `look.animations` is off. Reopening during a slide-out turns the
    /// drawer back from where it is.
    pub fn open(&mut self, look: &Look, tx: &mut Tx) {
        self.open_with(look.animations, tx);
    }

    /// Shows the preview with no slide, for state built before a window's first frame (a
    /// chooser that opens with a preview showing).
    pub fn open_at_once(&mut self) {
        self.open = true;
        self.marks = self.marks.wrapping_add(1);
    }

    pub(super) fn open_with(&mut self, animations: bool, tx: &mut Tx) {
        if !self.open {
            self.slide(animations, tx);
            // A fresh fit marker for this open, even when a close went before it in the same
            // handler: the host hears where suspension stands.
            self.marks = self.marks.wrapping_add(1);
        }
        self.open = true;
    }

    /// Hides the preview. Keyboard focus inside it returns to the files at once; the content
    /// fades out with the slide. Closing ends find and clears its query; the drawer tells the
    /// hosted view ([`find_end`](super::HostedPreview::find_end)) on its next build, so that
    /// build has to pass the view with [`PreviewDrawer::hosted`](super::PreviewDrawer::hosted)
    /// (a host that drops its view when nothing previews calls `find_end` itself).
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
        // `suspended` stays: the host last heard `Suspend(true)` if it was set, and the next
        // open's fit marker tells it otherwise.
        self.reset_find();
    }

    fn slide(&self, animations: bool, tx: &mut Tx) {
        if self.reserved || !animations {
            tx.instant();
            return;
        }
        // From rest: reopening mid-slide turns back where the drawer is, rather than carrying
        // on with the closing slide's momentum.
        let mut anim: Anim = Curve::tween(SLIDE_SECONDS, Easing::EASE_IN_OUT).into();
        anim.velocity = Velocity::Zero;
        tx.motion_curve(anim);
    }

    /// Lights the divider's line as hovering it does, or stops: C2 lights it while the pointer
    /// is over the edge of the column that meets the preview (`browser/view-modes/columns`:
    /// "Hovering the edge of the column that meets the preview also lights the preview divider
    /// line"). The grip's tint and the caption stay the divider's own hover's.
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

    /// Keeps the slot in Columns while the preview is closed, so opening and closing never
    /// move the columns, or releases it (Appearance → Preview panel off). Set it only while
    /// Columns shows; the host clears it in Icons and List. It is window-local and not a
    /// preference.
    pub fn reserve(&mut self, on: bool) {
        self.reserved = on;
    }

    /// The width the user set by dragging the divider or stepping it, if any. In Columns it
    /// is the session's minimum; in Icons and List it is the width.
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

    /// Whether the narrow-window rule hides the open preview's content. The host pauses
    /// media and defers loading while it does; the drawer reports each change with
    /// [`DrawerIntent::Suspend`](super::DrawerIntent::Suspend).
    pub fn is_suspended(&self) -> bool {
        self.open && self.suspended
    }

    /// Moves keyboard focus into the preview's content: its first focusable element, or the
    /// one marked `focus_selected`. Strata calls it for Right from the listing.
    ///
    /// Like the other focus moves, it works while focus is inside the split (the files or the
    /// drawer), and does nothing while nothing is focused: a `focus_return` moves focus only
    /// on behalf of a focused element (see the module docs).
    pub fn focus_content(&mut self) {
        self.focus_content = self.focus_content.wrapping_add(1);
    }

    /// Moves keyboard focus back to the files (their `focus_selected` row): Left at the
    /// content's edge, or at an archive's root.
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

    /// The find query, kept while the preview shows. A host that swaps the view (Rendered
    /// and Source) runs it on the new one.
    pub fn find_query(&self) -> &str {
        &self.find.query.text
    }

    /// Closes find and clears its query: another file is previewed, or the drawer closed. If
    /// the bar was open, the drawer tells the hosted view on its next build, as for
    /// [`DrawerState::close`]; a bar already closed told the view when it closed.
    pub fn reset_find(&mut self) {
        if self.find.open {
            self.find_end_pending = true;
        }
        self.find.open = false;
        self.find.query = TextEdit::default();
    }

    /// The preview's width for the sidebar's rail rule (C8, Strata): its manual width, at
    /// least `rules.hide_below`, or `rules.hide_below` without one.
    pub fn demand(&self, rules: &DrawerRules) -> f32 {
        self.manual.unwrap_or(0.0).max(rules.hide_below)
    }

    /// Opens find on `query`, selected, with the field focused: a host restoring find. The
    /// host runs the query on its view.
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
