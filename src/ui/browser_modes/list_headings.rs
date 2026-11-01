// SPDX-License-Identifier: MIT

//! The List table's heading row: sort buttons, resize handles, and drag to reorder.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{
    list_columns::{
        ListColumnDrag, ListColumnLayout, column_drop_feedback, column_drop_site,
        column_resize_handle, column_shown, find_column_at_x, register_list_column_cell,
    },
    pane::BoundItems,
};
use crate::{
    app::Browser,
    model::{SortDirection, SortKey, ViewPreferences},
    ui::preferences::{ListColumnId, PreferenceManager},
};

/// How far a heading press travels before it starts reordering columns.
const REORDER_THRESHOLD: f64 = 10.0;

pub(super) type ListSorting = Rc<HeadingSort>;

pub(super) struct HeadingSort {
    current: Cell<(SortKey, SortDirection)>,
    arrows: RefCell<Vec<(SortKey, gtk::Image)>>,
}

impl HeadingSort {
    pub(super) fn get(&self) -> (SortKey, SortDirection) {
        self.current.get()
    }

    pub(super) fn show(&self, key: SortKey, direction: SortDirection) {
        self.current.set((key, direction));
        for (arrow_key, arrow) in self.arrows.borrow().iter() {
            arrow.set_visible(*arrow_key == key);
            if *arrow_key == key {
                crate::assets::set_primary_icon(arrow, direction_icon(direction));
            }
        }
    }
}

fn direction_icon(direction: SortDirection) -> &'static str {
    if direction == SortDirection::Ascending {
        crate::assets::icons::ARROW_UP
    } else {
        crate::assets::icons::ARROW_DOWN
    }
}

pub(super) fn list_headings(
    browser: &Rc<Browser>,
    depth: usize,
    columns: ListColumnLayout,
    bound_items: &BoundItems,
) -> (gtk::Box, ListSorting) {
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("list-headings");
    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let sorting = Rc::new(HeadingSort {
        current: Cell::new((preferences.sort_key, preferences.sort_direction)),
        arrows: RefCell::default(),
    });

    let manager = PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();
    let cells = HeadingCells {
        browser: Rc::downgrade(browser),
        depth,
        columns: &columns,
        sorting: &sorting,
        preferences,
    };
    for col in order {
        headings.append(&cells.cell(col, column_shown(&hidden, col)));
    }

    install_column_reorder(&headings, &columns, bound_items);
    PreferenceManager::shared()
        .bind_interface_scale(&headings, move |_, scale| columns.rescale(scale));
    (headings, sorting)
}

/// What every heading cell of one table is built from.
struct HeadingCells<'a> {
    browser: Weak<Browser>,
    depth: usize,
    columns: &'a ListColumnLayout,
    sorting: &'a ListSorting,
    preferences: ViewPreferences,
}

impl HeadingCells<'_> {
    fn cell(&self, col: ListColumnId, is_visible: bool) -> gtk::Box {
        let index = col.index();
        let (text, key) = heading_label(col);
        let width = self.columns.widths[index].get();
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("list-heading-cell");
        cell.add_css_class(col.css_class());
        if !is_visible {
            cell.set_visible(false);
        }
        register_list_column_cell(self.columns, index, &cell);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let label = gtk::Label::new(Some(&crate::i18n::tr(text)));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(1);
        let arrow = crate::assets::primary_icon(direction_icon(self.preferences.sort_direction), 12);
        arrow.set_visible(key.is_some_and(|k| self.preferences.sort_key == k));
        row.append(&label);
        row.append(&arrow);
        let button = gtk::Button::builder().child(&row).build();
        button.add_css_class("list-heading-button");
        button.set_hexpand(true);
        if let Some(key) = key {
            self.connect_sort(&button, key, arrow);
        }

        let button_overlay = gtk::Overlay::new();
        button_overlay.set_child(Some(&button));
        button_overlay.set_hexpand(true);
        button_overlay.add_overlay(&column_resize_handle(
            self.columns.clone(),
            index,
            width,
            &cell,
        ));
        cell.append(&button_overlay);
        cell
    }

    fn connect_sort(&self, button: &gtk::Button, key: SortKey, arrow: gtk::Image) {
        button.set_cursor_from_name(Some("pointer"));
        let weak_browser = self.browser.clone();
        let sorting = Rc::downgrade(self.sorting);
        let depth = self.depth;
        button.connect_clicked(move |_| {
            let Some(sorting) = sorting.upgrade() else {
                return;
            };
            let direction = next_direction(sorting.get(), key);
            sorting.show(key, direction);
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_sort(depth, key, direction);
            }
        });
        self.sorting.arrows.borrow_mut().push((key, arrow));
    }
}

fn heading_label(col: ListColumnId) -> (&'static str, Option<SortKey>) {
    match col {
        ListColumnId::Name => ("Name", Some(SortKey::Name)),
        ListColumnId::Mode => ("Mode", None),
        ListColumnId::Size => ("Size", Some(SortKey::Size)),
        ListColumnId::Type => ("Type", Some(SortKey::Type)),
        ListColumnId::Modified => ("Modified", Some(SortKey::Modified)),
    }
}

/// A click on the sorted column flips its direction; another column sorts ascending.
fn next_direction(current: (SortKey, SortDirection), key: SortKey) -> SortDirection {
    let (current_key, current_direction) = current;
    if current_key != key {
        return SortDirection::Ascending;
    }
    match current_direction {
        SortDirection::Ascending => SortDirection::Descending,
        SortDirection::Descending => SortDirection::Ascending,
    }
}

fn install_column_reorder(headings: &gtk::Box, columns: &ListColumnLayout, bound_items: &BoundItems) {
    let drag = gtk::GestureDrag::new();
    drag.set_name(Some("column-drag"));
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let reorder = Rc::new(ColumnReorder {
        headings: headings.downgrade(),
        columns: columns.clone(),
        bound_items: bound_items.clone(),
    });
    drag.connect_drag_begin({
        let reorder = reorder.clone();
        move |gesture, start_x, start_y| reorder.begin(gesture, start_x, start_y)
    });
    drag.connect_drag_update({
        let reorder = reorder.clone();
        move |gesture, offset_x, offset_y| reorder.update(gesture, offset_x, offset_y)
    });
    drag.connect_drag_end({
        let reorder = reorder.clone();
        move |_, x, y| reorder.finish(Some((x, y)))
    });
    drag.connect_cancel({
        let reorder = reorder.clone();
        move |_, _| reorder.finish(None)
    });
    headings.connect_unmap(move |_| reorder.finish(None));
    headings.add_controller(drag);
}

/// Dragging a heading sideways moves its column.
struct ColumnReorder {
    headings: glib::WeakRef<gtk::Box>,
    columns: ListColumnLayout,
    bound_items: BoundItems,
}

impl ColumnReorder {
    fn finish(&self, released: Option<(f64, f64)>) {
        if let Some(headings) = self.headings.upgrade() {
            self.columns
                .finish_drag(&headings, &self.bound_items, released);
        } else {
            self.columns.drag.take();
        }
    }

    fn begin(&self, gesture: &gtk::GestureDrag, start_x: f64, start_y: f64) {
        self.finish(None);
        let Some(headings) = self.headings.upgrade() else {
            return;
        };
        let Some((column, is_resize)) = find_column_at_x(&headings, start_x, &self.columns) else {
            return;
        };
        if is_resize {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }
        self.columns.drag.set(Some(ListColumnDrag {
            column,
            origin: (start_x, start_y),
            active: false,
        }));
    }

    fn update(&self, gesture: &gtk::GestureDrag, offset_x: f64, offset_y: f64) {
        let Some(mut drag) = self.columns.drag.get() else {
            return;
        };
        let Some(headings) = self.headings.upgrade() else {
            return;
        };
        if !drag.active {
            if offset_x.abs() < REORDER_THRESHOLD {
                return;
            }
            drag.active = true;
            self.columns.drag.set(Some(drag));
            gesture.set_state(gtk::EventSequenceState::Claimed);
            headings.set_cursor_from_name(Some("grabbing"));
            headings.add_css_class("column-reordering");
        }
        column_drop_feedback(
            &headings,
            column_drop_site(&headings, &self.columns, drag, (offset_x, offset_y)),
        );
    }
}
