// SPDX-License-Identifier: MIT

//! 10xer selection among results that replace the listing. Results keep their
//! own selection, apart from the hidden directory's, so a fill made with
//! **Space** or **v** / **V** is the results' selection. Motion over a fill
//! moves only the keyboard cursor. A selection changed any other way, such as
//! by the pointer or a new query, ends the fill and its range; the next fill
//! starts from that selection unless it is only the cursor.

use std::cell::RefCell;

use gtk::{glib, prelude::*};

use super::{
    BrowserView, ViewState,
    listing_filter::{Hits, Target, results_step_target, scroll_results_to},
};
use crate::app::VisualKind;

struct Range {
    kind: VisualKind,
    anchor: u32,
    /// The fill the range walks over; walking back restores it.
    base: gtk::Bitset,
}

struct Fill {
    owner: glib::WeakRef<gtk::Entry>,
    /// The selection this fill last wrote.
    written: gtk::Bitset,
    range: Option<Range>,
}

#[derive(Default)]
pub(super) struct ResultSelection {
    fill: RefCell<Option<Fill>>,
}

/// The selection a new fill starts from. Outside a fill the cursor alone is
/// selected without being part of any fill.
fn adopted(hits: &Hits) -> gtk::Bitset {
    let selected = hits.selection.selection();
    let cursor_only = selected.size() == 1 && hits.cursor.is_some_and(|at| selected.contains(at));
    if cursor_only {
        gtk::Bitset::new_empty()
    } else {
        selected
    }
}

fn focus_hit(hits: &Hits, position: u32) {
    hits.view.grab_focus();
    scroll_results_to(&hits.view, position, gtk::ListScrollFlags::FOCUS);
}

fn write(hits: &Hits, fill: &mut Fill, selected: gtk::Bitset) {
    hits.selection
        .set_selection(&selected, &gtk::Bitset::new_range(0, hits.count()));
    fill.written = selected;
}

/// Rewrites the range from its anchor to `cursor` over its base fill.
fn walk(hits: &Hits, fill: &mut Fill, cursor: u32) {
    let Some(range) = fill.range.as_ref() else {
        return;
    };
    let (start, end) = (range.anchor.min(cursor), range.anchor.max(cursor));
    let span = gtk::Bitset::new_range(start, end - start + 1);
    let selected = range.base.copy();
    match range.kind {
        VisualKind::Select => selected.union(&span),
        VisualKind::Unset => selected.subtract(&span),
    }
    write(hits, fill, selected);
}

impl ViewState {
    /// Takes the fill that still owns the showing results' selection, with
    /// those results. A stale fill is dropped. Selection handlers may run
    /// while the fill is out, so callers put it back with
    /// [`Self::keep_result_fill`] after writing.
    fn take_result_fill(&self) -> Option<(Target, Hits, Fill)> {
        let fill = self.result_selection.fill.take()?;
        let target = self.filter_target()?;
        let hits = target.hits()?;
        let live = fill.owner.upgrade().as_ref() == Some(target.entry())
            && fill.written.equals(&hits.selection.selection());
        live.then_some((target, hits, fill))
    }

    fn keep_result_fill(&self, fill: Fill) {
        self.result_selection.fill.replace(Some(fill));
    }

    /// Returns keyboard focus to the cursor without rewriting a fill.
    /// Returns `false` without one.
    pub(super) fn focus_filled_results(&self) -> bool {
        let Some((_, hits, fill)) = self.take_result_fill() else {
            return false;
        };
        self.keep_result_fill(fill);
        hits.view.grab_focus();
        true
    }

    pub(super) fn forget_result_fill(&self) {
        self.result_selection.fill.take();
    }

    pub(super) fn result_visual_kind(&self) -> Option<VisualKind> {
        let (_, _, fill) = self.take_result_fill()?;
        let kind = fill.range.as_ref().map(|range| range.kind);
        self.keep_result_fill(fill);
        kind
    }
}

impl BrowserView {
    /// Moves the cursor over a filled result list without rewriting its fill,
    /// extending a range. Returns `false` without a fill.
    pub(super) fn step_filled_results(&self, direction: i32, steps: usize) -> bool {
        let Some((_, hits, mut fill)) = self.state.take_result_fill() else {
            return false;
        };
        if let Some(cursor) = results_step_target(hits.cursor, hits.count(), direction, steps) {
            focus_hit(&hits, cursor);
            walk(&hits, &mut fill, cursor);
        }
        self.state.keep_result_fill(fill);
        true
    }

    /// Runs a native grid move that selects the cell it reaches, then puts
    /// back the fill, extending a range to the new cursor.
    pub(in crate::ui) fn keep_result_fill(&self, motion: impl FnOnce()) {
        let Some((_, _, mut fill)) = self.state.take_result_fill() else {
            motion();
            return;
        };
        motion();
        let Some(hits) = self.filter_target().and_then(|target| target.hits()) else {
            return;
        };
        match hits.cursor {
            Some(cursor) if fill.range.is_some() => walk(&hits, &mut fill, cursor),
            _ => {
                let written = fill.written.copy();
                write(&hits, &mut fill, written);
            }
        }
        self.state.keep_result_fill(fill);
    }

    /// **Space** on results: toggles the cursor hit into the fill and moves
    /// down, or toggles it within a range without moving. A cursor-only hit
    /// is added, never removed. `None` while no results replace the listing;
    /// `Some(false)` without a hit.
    pub(in crate::ui) fn toggle_result_and_advance(&self) -> Option<bool> {
        let hits = self.filter_target()?.hits()?;
        self.keyboard_navigation();
        let count = hits.count();
        if count == 0 {
            return Some(false);
        }
        let cursor = hits.cursor.unwrap_or(0).min(count - 1);
        let mut fill = match self.state.take_result_fill() {
            Some((_, _, fill)) => fill,
            None => Fill {
                owner: self.filter_target()?.entry().downgrade(),
                written: adopted(&hits),
                range: None,
            },
        };
        let toggled = |selected: &gtk::Bitset| {
            let selected = selected.copy();
            if selected.contains(cursor) {
                selected.remove(cursor);
            } else {
                selected.add(cursor);
            }
            selected
        };
        let ranged = if let Some(range) = fill.range.as_mut() {
            range.base = toggled(&range.base);
            walk(&hits, &mut fill, cursor);
            true
        } else {
            let selected = toggled(&fill.written);
            write(&hits, &mut fill, selected);
            false
        };
        self.state.keep_result_fill(fill);
        if !ranged {
            let next = results_step_target(Some(cursor), count, 1, 1).unwrap_or(cursor);
            focus_hit(&hits, next);
        }
        Some(true)
    }

    /// **v** / **V** on results: the same key leaves the range and keeps its
    /// fill; otherwise a new range starts at the cursor over the current fill.
    /// `None` while no results replace the listing; `Some(false)` without a hit.
    pub(in crate::ui) fn toggle_result_visual(&self, kind: VisualKind) -> Option<bool> {
        let target = self.filter_target()?;
        let hits = target.hits()?;
        self.keyboard_navigation();
        let count = hits.count();
        if count == 0 {
            return Some(false);
        }
        let cursor = hits.cursor.unwrap_or(0).min(count - 1);
        let base = match self.state.take_result_fill() {
            Some((_, _, mut fill)) => {
                if fill.range.as_ref().is_some_and(|range| range.kind == kind) {
                    fill.range = None;
                    self.state.keep_result_fill(fill);
                    self.state.notify_search_selection_changed();
                    return Some(true);
                }
                fill.written
            }
            None => adopted(&hits),
        };
        let mut fill = Fill {
            owner: target.entry().downgrade(),
            written: gtk::Bitset::new_empty(),
            range: Some(Range {
                kind,
                anchor: cursor,
                base,
            }),
        };
        walk(&hits, &mut fill, cursor);
        self.state.keep_result_fill(fill);
        focus_hit(&hits, cursor);
        self.state.notify_search_selection_changed();
        Some(true)
    }

    /// Ends a range over results and keeps its fill.
    pub(in crate::ui) fn leave_result_visual(&self) -> bool {
        let Some((_, _, mut fill)) = self.state.take_result_fill() else {
            return false;
        };
        let ranged = fill.range.take().is_some();
        self.state.keep_result_fill(fill);
        if ranged {
            self.state.notify_search_selection_changed();
        }
        ranged
    }
}
