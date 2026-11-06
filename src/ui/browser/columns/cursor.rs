// SPDX-License-Identifier: MIT

//! Scrolling a column to a row and restoring keyboard focus to its cursor row.

use super::{BoundRow, ColumnView};
use crate::ui::browser::collection::scroll_collection_when_allocated;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How many frames a cursor restore waits for its row to be mapped.
const CURSOR_RESTORE_FRAMES: u8 = 8;

pub(in crate::ui::browser) fn scroll_column_to(column: &ColumnView, position: u32) {
    if position >= column.selection.n_items() {
        return;
    }
    scroll_collection_when_allocated(column.list.upcast_ref(), position);
}

/// `is_ancestor` is true when the receiver sits inside the argument.
/// Restore while focus is the list or one of its rows, while a parent of
/// the list still holds focus, and while focus is on a row the update
/// removed. An inline name editor also sits inside a row and must keep the
/// focus it took after this queued.
fn list_keeps_focus(list: &gtk::ListView, focused: &gtk::Widget) -> bool {
    let near_list = focused == list
        || focused.is_ancestor(list)
        || list.is_ancestor(focused)
        || crate::ui::focus_navigation::focus_removed_from(list.upcast_ref(), focused);
    near_list && !crate::ui::focus_navigation::editable(focused)
}

fn mapped_cursor_row(rows: &RefCell<Vec<BoundRow>>, position: u32) -> Option<gtk::Widget> {
    rows.borrow().iter().find_map(|bound| {
        let item = bound.item.upgrade()?;
        (item.position() == position)
            .then(|| bound.row.upgrade()?.parent())
            .flatten()
            .filter(|cursor| cursor.is_mapped())
    })
}

/// One queued restore of the cursor at `position`.
struct CursorRestore {
    rows: Rc<RefCell<Vec<BoundRow>>>,
    generations: Rc<Cell<u64>>,
    generation: u64,
    /// Recursive hits that replaced the rows since keep their own cursor.
    hits: Rc<Cell<bool>>,
    position: u32,
    frames: Cell<u8>,
}

impl CursorRestore {
    fn tick(&self, list: &gtk::ListView) -> glib::ControlFlow {
        if self.generations.get() != self.generation || self.hits.get() {
            return glib::ControlFlow::Break;
        }
        let focused = list.root().and_then(|root| root.focus());
        if !focused
            .as_ref()
            .is_some_and(|focused| list_keeps_focus(list, focused))
        {
            return glib::ControlFlow::Break;
        }
        if let Some(cursor) = mapped_cursor_row(&self.rows, self.position) {
            cursor.grab_focus();
            return glib::ControlFlow::Break;
        }
        self.frames.set(self.frames.get().saturating_add(1));
        if self.frames.get() >= CURSOR_RESTORE_FRAMES {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }
}

pub(in crate::ui::browser) fn restore_column_cursor(column: &ColumnView, position: u32) {
    let list = column.list.downgrade();
    let generations = column.cursor_restore_generation.clone();
    let generation = generations.get().wrapping_add(1);
    generations.set(generation);
    let restore = CursorRestore {
        rows: column.bound_rows.clone(),
        generations,
        generation,
        hits: column.recursive_search_active.clone(),
        position,
        frames: Cell::new(0),
    };
    glib::idle_add_local_once(move || {
        let Some(list) = list.upgrade() else { return };
        list.add_tick_callback(move |list, _| restore.tick(list));
    });
}
