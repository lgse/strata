// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};

use super::rows::materialize_row;
use super::tags::document_tag_table;
use super::{BoundRow, VIRTUAL_ROW_MIN_HEIGHT, VirtualPreviewState};

/// Source rows build on bind; rendered document rows build once mapped and
/// share one text tag table.
#[derive(Clone)]
struct RowKind {
    source: bool,
    document_tags: Option<gtk::TextTagTable>,
}

pub(super) fn row_factory(
    state: &Rc<VirtualPreviewState>,
    source: bool,
) -> gtk::SignalListItemFactory {
    let rows = RowKind {
        source,
        document_tags: (!source).then(document_tag_table),
    };
    let factory = gtk::SignalListItemFactory::new();
    let state_for_map = Rc::downgrade(state);
    let rows_for_setup = rows.clone();
    factory.connect_setup(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            setup_row(item, &rows_for_setup, &state_for_map);
        }
    });

    let state_for_bind = state.clone();
    factory.connect_bind(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            bind_row(item, &rows, &state_for_bind);
        }
    });

    let state_for_unbind = state.clone();
    factory.connect_unbind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        state_for_unbind
            .bound
            .borrow_mut()
            .remove(&(item.position() as usize));
    });
    factory
}

fn setup_row(item: &gtk::ListItem, rows: &RowKind, state: &Weak<VirtualPreviewState>) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.add_css_class("preview-virtual-row");
    row.set_size_request(-1, VIRTUAL_ROW_MIN_HEIGHT);
    row.set_hexpand(true);
    if !rows.source {
        row.set_layout_manager(Some(
            super::super::preview::media_layout::ContentColumn::new(),
        ));
        materialize_on_map(&row, item, state, rows.document_tags.clone());
    }
    item.set_child(Some(&row));
}

fn materialize_on_map(
    row: &gtk::Box,
    item: &gtk::ListItem,
    state: &Weak<VirtualPreviewState>,
    document_tags: Option<gtk::TextTagTable>,
) {
    let weak_item = item.downgrade();
    let state = state.clone();
    row.connect_map(move |row| {
        if let (Some(item), Some(state)) = (weak_item.upgrade(), state.upgrade()) {
            materialize_row(
                row,
                item.position() as usize,
                &state,
                document_tags.as_ref(),
            );
            // GTK drops a resize queued while it lays this row out; the next frame keeps it.
            row.add_tick_callback(|row, _| {
                row.queue_resize();
                glib::ControlFlow::Break
            });
        }
    });
}

fn bind_row(item: &gtk::ListItem, rows: &RowKind, state: &Rc<VirtualPreviewState>) {
    let Some(row) = item.child().and_downcast::<gtk::Box>() else {
        return;
    };
    let index = item.position() as usize;
    let bound = BoundRow::default();
    bound.root.set(Some(&row));
    state.bound.borrow_mut().insert(index, bound);
    if rows.source {
        materialize_row(&row, index, state, None);
    } else if row.is_mapped() {
        materialize_next_tick(&row, item, state, rows.document_tags.clone());
    }
}

/// GTK rebinds rows before unmapping its old prefetch window. Discard
/// intermediate bindings before building their widget trees.
fn materialize_next_tick(
    row: &gtk::Box,
    item: &gtk::ListItem,
    state: &Rc<VirtualPreviewState>,
    document_tags: Option<gtk::TextTagTable>,
) {
    let index = item.position() as usize;
    let weak_state = Rc::downgrade(state);
    let weak_item = item.downgrade();
    row.add_tick_callback(move |row, _| {
        let Some(state) = weak_state.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let still_bound = weak_item
            .upgrade()
            .is_some_and(|item| item.position() as usize == index);
        if still_bound && row.is_mapped() {
            materialize_row(row, index, &state, document_tags.as_ref());
        }
        glib::ControlFlow::Break
    });
}
