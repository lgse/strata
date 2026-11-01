// SPDX-License-Identifier: MIT

//! The file-row factory of one column: row widgets, pointer gestures and binding.

use super::{BoundRow, ColumnHits, PendingPointerActivation};
use crate::model::FileEntry;
use crate::services::SearchItem;
use crate::ui::browser::{ViewState, collection::ViewMap};
use crate::ui::enter_target::EnterTarget;
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

mod bind;
mod click;
mod drag;
mod widgets;

pub(super) struct ColumnRows {
    pub(super) factory: gtk::SignalListItemFactory,
    pub(super) bound_rows: Rc<RefCell<Vec<BoundRow>>>,
    pub(super) scrolling: Rc<Cell<bool>>,
}

/// Each column's row factory binds to that column's own models and state.
pub(super) struct RowSources<'a> {
    pub(super) depth: usize,
    pub(super) map: &'a ViewMap,
    pub(super) selection: &'a gtk::MultiSelection,
    pub(super) recursive_search_active: &'a Rc<Cell<bool>>,
    pub(super) search_results: &'a Rc<RefCell<Vec<SearchItem>>>,
    pub(super) hits: &'a Rc<RefCell<ColumnHits>>,
    pub(super) enter_target: &'a EnterTarget,
}

/// The column state every row's gestures read.
struct RowContext {
    state: Weak<ViewState>,
    depth: usize,
    map: ViewMap,
    selection: gtk::MultiSelection,
    search_active: Rc<Cell<bool>>,
    search_results: Rc<RefCell<Vec<SearchItem>>>,
}

impl RowContext {
    fn search_entry(&self, position: u32) -> Option<FileEntry> {
        self.search_results
            .borrow()
            .get(position as usize)
            .map(crate::ui::browser::search_result_entry)
    }

    /// The entry at a view position, from the search hits or the column's listing.
    fn view_entry(&self, state: &ViewState, position: u32) -> Option<FileEntry> {
        if self.search_active.get() {
            self.search_entry(position)
        } else {
            self.map
                .source_position(position)
                .and_then(|source_position| state.browser.entry_at(self.depth, source_position))
        }
    }
}

/// Pointer state one row shares between its click and drag gestures.
#[derive(Clone, Default)]
struct RowPointer {
    pending_activation: Rc<RefCell<Option<PendingPointerActivation>>>,
    was_selected: Rc<Cell<bool>>,
    press_origin: Rc<Cell<(f64, f64)>>,
}

/// One row as its gesture handlers see it.
#[derive(Clone)]
struct RowHandle {
    context: Rc<RowContext>,
    item: glib::WeakRef<gtk::ListItem>,
    row: glib::WeakRef<gtk::Box>,
    pointer: RowPointer,
}

pub(super) fn column_rows(state: &Rc<ViewState>, sources: &RowSources<'_>) -> ColumnRows {
    let scrolling = Rc::new(Cell::new(false));
    let factory = gtk::SignalListItemFactory::new();
    let bound_rows: Rc<RefCell<Vec<BoundRow>>> = Rc::new(RefCell::new(Vec::new()));
    let context = Rc::new(RowContext {
        state: Rc::downgrade(state),
        depth: sources.depth,
        map: sources.map.clone(),
        selection: sources.selection.clone(),
        search_active: sources.recursive_search_active.clone(),
        search_results: sources.search_results.clone(),
    });
    let rows_for_setup = bound_rows.clone();
    let setup_context = context.clone();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let bound = setup_row(&setup_context, item);
        rows_for_setup.borrow_mut().push(bound);
    });
    let bind_context = bind::BindContext {
        rows: context,
        hits: sources.hits.clone(),
        enter_target: sources.enter_target.clone(),
        scrolling: scrolling.clone(),
    };
    factory.connect_bind(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            bind_context.bind(item);
        }
    });
    factory.connect_unbind(move |_, object| {
        if let Some(edit) = crate::ui::collection_edit::item_edit(object) {
            edit.unbind();
        }
        crate::ui::thumbnail::cancel_list_item_thumbnails(object);
    });
    let rows_for_teardown = bound_rows.clone();
    factory.connect_teardown(move |_, object| {
        crate::ui::collection_edit::forget_item_edit(object);
        rows_for_teardown.borrow_mut().retain(|bound| {
            bound
                .item
                .upgrade()
                .is_some_and(|item| item.upcast_ref::<glib::Object>() != object)
        });
    });
    ColumnRows {
        factory,
        bound_rows,
        scrolling,
    }
}

fn setup_row(context: &Rc<RowContext>, item: &gtk::ListItem) -> BoundRow {
    let widgets = widgets::RowWidgets::build();
    let row = &widgets.row;
    item.set_child(Some(row));
    let handle = RowHandle {
        context: context.clone(),
        item: item.downgrade(),
        row: row.downgrade(),
        pointer: RowPointer::default(),
    };
    let pending_activation_for_unmap = handle.pointer.pending_activation.clone();
    row.connect_unmap(move |_| {
        pending_activation_for_unmap.take();
    });
    let interactive = context
        .state
        .upgrade()
        .is_some_and(|state| state.interactive);
    let content_drag = interactive.then(|| {
        let drag = handle.install_drag_source(row);
        handle.install_drop_target(row);
        drag
    });
    let selection_click = click::RowClick::install(&handle, row, &widgets.label);
    if let Some(drag) = &content_drag {
        drag.group_with(&selection_click);
    }
    let middle_context = context.clone();
    crate::ui::collection_interaction::install_row_middle_click(
        row.upcast_ref::<gtk::Widget>(),
        item,
        move |position, modifiers| click::open_middle_clicked(&middle_context, position, modifiers),
    );
    widgets.bound_row(item)
}
