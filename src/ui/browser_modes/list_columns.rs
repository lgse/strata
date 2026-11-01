// SPDX-License-Identifier: MIT

//! The List table's columns: their widths, order, and visibility, and the header
//! gestures that resize and reorder them.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::{BrowserMode, ModeViews, column_options, list_factory::ListRow, pane::BoundItems};
use crate::{
    app::Browser,
    ui::preferences::{ListColumnId, ListColumns, PreferenceManager},
};

const LIST_COLUMN_WIDTHS: [i32; 5] = [160, 160, 90, 120, 150];
const LIST_COLUMN_MIN_WIDTHS: [i32; 5] = [160, 80, 70, 80, 110];
/// How close to a heading's trailing edge a press resizes rather than reorders.
const RESIZE_EDGE: f64 = 8.0;

#[derive(Clone)]
pub(super) struct ListColumnLayout {
    pub(super) widths: Rc<Vec<Cell<i32>>>,
    pub(super) cells: Rc<Vec<RefCell<Vec<glib::WeakRef<gtk::Widget>>>>>,
    pub(super) name_manually_resized: Rc<Cell<bool>>,
    pub(super) scale: Rc<Cell<f64>>,
    pub(super) drag: Rc<Cell<Option<ListColumnDrag>>>,
    chooser: bool,
}

impl ListColumnLayout {
    pub(super) fn new(browser: &Browser) -> Self {
        let chooser = browser.is_chooser_mode();
        let preferences = PreferenceManager::shared();
        let saved = if chooser {
            preferences.chooser_list_columns()
        } else {
            preferences.browser_list_columns()
        };
        let widths = saved.map_or(LIST_COLUMN_WIDTHS, saved_widths);
        Self {
            widths: Rc::new(widths.into_iter().map(Cell::new).collect()),
            cells: Rc::new((0..5).map(|_| RefCell::new(Vec::new())).collect()),
            name_manually_resized: Rc::new(Cell::new(
                saved.is_some_and(|saved| saved.name.is_some()),
            )),
            scale: Rc::new(Cell::new(1.0)),
            drag: Rc::default(),
            chooser,
        }
    }

    pub(super) fn finish_drag(
        &self,
        headings: &gtk::Box,
        bound_items: &BoundItems,
        released: Option<(f64, f64)>,
    ) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        headings.set_cursor(None);
        headings.remove_css_class("column-reordering");
        column_drop_feedback(headings, None);
        if !drag.active {
            return;
        }
        let Some((target, after)) =
            released.and_then(|offset| column_drop_site(headings, self, drag, offset))
        else {
            return;
        };
        PreferenceManager::shared().move_list_column(drag.column, target, after);
        apply_columns_layout(headings, self, bound_items);
    }

    pub(super) fn remember(&self) {
        let scale = self.scale.get();
        let unscaled = |index: usize| (f64::from(self.widths[index].get()) / scale).round() as i32;
        let saved = Some(ListColumns {
            name: self.name_manually_resized.get().then(|| unscaled(0)),
            mode: unscaled(1),
            size: unscaled(2),
            kind: unscaled(3),
            modified: unscaled(4),
        });
        let preferences = PreferenceManager::shared();
        if self.chooser {
            preferences.set_chooser_list_columns(saved);
        } else {
            preferences.set_browser_list_columns(saved);
        }
    }

    /// Scales every column after an interface-scale change.
    pub(super) fn rescale(&self, scale: f64) {
        let ratio = scale / self.scale.replace(scale);
        for (index, width) in self.widths.iter().enumerate() {
            let scaled = (f64::from(width.get()) * ratio).round() as i32;
            width.set(scaled);
            self.cells[index].borrow_mut().retain(|weak| {
                let Some(cell) = weak.upgrade() else {
                    return false;
                };
                cell.set_width_request(scaled);
                true
            });
        }
    }

    /// The widest content any cell of the column has.
    fn natural_width(&self, index: usize, fallback: i32) -> i32 {
        self.cells[index]
            .borrow()
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .map(|widget| list_cell_content_width(&widget))
            .max()
            .unwrap_or(fallback)
    }
}

fn saved_widths(saved: ListColumns) -> [i32; 5] {
    let mut widths = [
        saved.name.unwrap_or(LIST_COLUMN_WIDTHS[0]),
        saved.mode,
        saved.size,
        saved.kind,
        saved.modified,
    ];
    for (index, width) in widths.iter_mut().enumerate() {
        *width = list_column_width(index, *width);
    }
    widths
}

/// Name stays shown even when the preferences hide it.
pub(super) fn column_shown(hidden: &[ListColumnId], column: ListColumnId) -> bool {
    !hidden.contains(&column) || column == ListColumnId::Name
}

#[derive(Clone, Copy)]
pub(super) struct ListColumnDrag {
    pub(super) column: ListColumnId,
    pub(super) origin: (f64, f64),
    pub(super) active: bool,
}

pub(super) fn find_heading_cell(headings: &gtk::Box, col: ListColumnId) -> Option<gtk::Widget> {
    let class = col.css_class();
    let mut current = headings.first_child();
    while let Some(child) = current {
        if child.has_css_class(class) {
            return Some(child);
        }
        current = child.next_sibling();
    }
    None
}

/// The heading cell's allocated left edge and width, once it has a width.
fn heading_extent(headings: &gtk::Box, col: ListColumnId) -> Option<(f64, f64)> {
    let bounds = find_heading_cell(headings, col)?.compute_bounds(headings)?;
    (bounds.width() > 0.0).then(|| (f64::from(bounds.x()), f64::from(bounds.width())))
}

/// The column spanning `x` from `left` to `right`, and whether `x` is on its resize edge.
fn column_hit(col: ListColumnId, left: f64, right: f64, x: f64) -> Option<(ListColumnId, bool)> {
    (x >= left && x <= right).then_some((col, x >= (right - RESIZE_EDGE)))
}

pub(super) fn find_column_at_x(
    headings: &gtk::Box,
    x: f64,
    columns: &ListColumnLayout,
) -> Option<(ListColumnId, bool)> {
    let visible_cols = PreferenceManager::shared().list_visible_columns();
    visible_cols
        .iter()
        .find_map(|&col| {
            let (left, width) = heading_extent(headings, col)?;
            column_hit(col, left, left + width, x)
        })
        .or_else(|| layout_column_at_x(&visible_cols, x, columns))
}

/// Before headings are allocated, the column under `x` by the stored widths.
fn layout_column_at_x(
    visible_cols: &[ListColumnId],
    x: f64,
    columns: &ListColumnLayout,
) -> Option<(ListColumnId, bool)> {
    let mut x_acc = 0.0;
    for &col in visible_cols {
        let w = f64::from(columns.widths[col.index()].get());
        if let Some(hit) = column_hit(col, x_acc, x_acc + w, x) {
            return Some(hit);
        }
        x_acc += w;
    }
    None
}

fn column_midpoint(
    headings: &gtk::Box,
    col: ListColumnId,
    columns: &ListColumnLayout,
) -> Option<f64> {
    if let Some((left, width)) = heading_extent(headings, col) {
        return Some(left + (width / 2.0));
    }
    let mut x_acc = 0.0;
    for c in PreferenceManager::shared().list_visible_columns() {
        let w = f64::from(columns.widths[c.index()].get());
        if c == col {
            return Some(x_acc + w / 2.0);
        }
        x_acc += w;
    }
    None
}

fn outside(headings: &gtk::Box, x: f64, y: f64) -> bool {
    let outside_x = x < 0.0 || x > f64::from(headings.width());
    let outside_y = y < 0.0 || y > f64::from(headings.height());
    outside_x || outside_y
}

pub(super) fn column_drop_site(
    headings: &gtk::Box,
    columns: &ListColumnLayout,
    drag: ListColumnDrag,
    offset: (f64, f64),
) -> Option<(ListColumnId, bool)> {
    let (x, y) = (drag.origin.0 + offset.0, drag.origin.1 + offset.1);
    if outside(headings, x, y) {
        return None;
    }
    let mut visible = PreferenceManager::shared().list_visible_columns();
    let original = visible.iter().position(|column| *column == drag.column)?;
    visible.remove(original);
    for (position, column) in visible.iter().copied().enumerate() {
        if x < column_midpoint(headings, column, columns)? {
            return (position != original).then_some((column, false));
        }
    }
    let last = visible.last().copied()?;
    (visible.len() != original).then_some((last, true))
}

pub(super) fn column_drop_feedback(headings: &gtk::Box, destination: Option<(ListColumnId, bool)>) {
    for column in ListColumnId::ALL {
        let Some(cell) = find_heading_cell(headings, column) else {
            continue;
        };
        cell.remove_css_class("reorder-before");
        cell.remove_css_class("reorder-after");
        match destination {
            Some((target, after)) if target == column => cell.add_css_class(if after {
                "reorder-after"
            } else {
                "reorder-before"
            }),
            _ => {}
        }
    }
}

pub(super) fn register_list_column_cell(
    columns: &ListColumnLayout,
    index: usize,
    widget: &impl IsA<gtk::Widget>,
) {
    widget.set_width_request(columns.widths[index].get());
    // Until the user resizes it, Name absorbs space left after the fixed metadata columns.
    widget.set_hexpand(index == 0 && !columns.name_manually_resized.get());
    let weak = glib::WeakRef::new();
    weak.set(Some(widget.upcast_ref()));
    columns.cells[index].borrow_mut().push(weak);
}

fn set_list_column_width(columns: &ListColumnLayout, index: usize, width: i32) {
    columns.widths[index].set(width);
    if index == 0 {
        columns.name_manually_resized.set(true);
    }
    columns.cells[index].borrow_mut().retain(|weak| {
        let Some(widget) = weak.upgrade() else {
            return false;
        };
        widget.set_width_request(width);
        if index == 0 {
            widget.set_hexpand(false);
        }
        true
    });
}

/// A heading's resize drag: a double press autofits, a drag follows the pointer.
struct ColumnResize {
    columns: ListColumnLayout,
    index: usize,
    initial_width: i32,
    heading: glib::WeakRef<gtk::Box>,
    starting_width: Cell<i32>,
    pointer_start: Cell<Option<f64>>,
    last_press: Cell<u64>,
}

impl ColumnResize {
    fn begin(&self, gesture: &gtk::GestureDrag) {
        let now = glib::monotonic_time() as u64;
        let prev = self.last_press.replace(now);
        if now.wrapping_sub(prev) <= 400_000 {
            let natural = self.columns.natural_width(self.index, self.initial_width);
            set_list_column_width(
                &self.columns,
                self.index,
                list_column_width(self.index, natural),
            );
            self.columns.remember();
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }
        // Registered cells also include unmapped loading placeholders with stale allocations.
        let width = self
            .heading
            .upgrade()
            .and_then(|widget| widget.compute_bounds(&widget))
            .map_or(self.initial_width, |bounds| bounds.width().round() as i32);
        self.starting_width
            .set(list_column_width(self.index, width));
        self.pointer_start.set(pointer_x(gesture));
        gesture.set_state(gtk::EventSequenceState::Claimed);
    }

    fn update(&self, gesture: &gtk::GestureDrag, fallback_offset_x: f64) {
        let offset_x = self
            .pointer_start
            .get()
            .zip(pointer_x(gesture))
            .map_or(fallback_offset_x, |(start, current)| current - start);
        let width = (f64::from(self.starting_width.get()) + offset_x).round() as i32;
        set_list_column_width(
            &self.columns,
            self.index,
            list_column_width(self.index, width),
        );
    }
}

fn pointer_x(gesture: &gtk::GestureDrag) -> Option<f64> {
    gesture
        .current_event()
        .and_then(|event| event.position())
        .map(|(pointer_x, _)| pointer_x)
}

pub(super) fn column_resize_handle(
    columns: ListColumnLayout,
    index: usize,
    initial_width: i32,
    heading: &gtk::Box,
) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    handle.add_css_class("list-column-resize-handle");
    handle.set_width_request(7);
    handle.set_halign(gtk::Align::End);
    handle.set_valign(gtk::Align::Fill);
    handle.set_cursor_from_name(Some("col-resize"));
    let gesture = gtk::GestureDrag::new();
    gesture.set_button(1);
    let resize = Rc::new(ColumnResize {
        columns: columns.clone(),
        index,
        initial_width,
        heading: heading.downgrade(),
        starting_width: Cell::new(initial_width),
        pointer_start: Cell::new(None),
        last_press: Cell::new(0),
    });
    let begin = resize.clone();
    gesture.connect_drag_begin(move |gesture, _, _| begin.begin(gesture));
    gesture.connect_drag_update(move |gesture, offset_x, _| resize.update(gesture, offset_x));
    gesture.connect_drag_end(move |_, _, _| columns.remember());
    handle.add_controller(gesture);
    handle
}

fn list_column_width(index: usize, width: i32) -> i32 {
    width.max(LIST_COLUMN_MIN_WIDTHS[index])
}

/// Every label under `cell` whose natural width is capped.
fn capped_labels(cell: &gtk::Widget) -> Vec<(gtk::Label, i32)> {
    let mut labels = Vec::new();
    let mut stack = vec![cell.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget
            .downcast_ref::<gtk::Label>()
            .filter(|label| label.max_width_chars() >= 0)
        {
            labels.push((label.clone(), label.max_width_chars()));
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            stack.push(c.clone());
            child = c.next_sibling();
        }
    }
    labels
}

/// Registered cells carry the column's fixed width and their labels cap natural
/// width at one character, so autofit must measure with both constraints lifted.
fn list_cell_content_width(cell: &gtk::Widget) -> i32 {
    let cell_request = cell.width_request();
    if cell_request >= 0 {
        cell.set_width_request(-1);
    }
    let labels = capped_labels(cell);
    for (label, _) in &labels {
        label.set_max_width_chars(-1);
    }
    let (_, natural, _, _) = cell.measure(gtk::Orientation::Horizontal, -1);
    if cell_request >= 0 {
        cell.set_width_request(cell_request);
    }
    for (label, chars) in labels {
        label.set_max_width_chars(chars);
    }
    natural
}

pub(super) fn apply_columns_layout(
    headings: &gtk::Box,
    columns: &ListColumnLayout,
    bound_items: &BoundItems,
) {
    let manager = PreferenceManager::shared();
    let order = manager.list_column_order();
    let hidden = manager.list_hidden_columns();
    show_shown_columns(columns, &hidden);
    reorder_headings(headings, &order);
    for bound in bound_items.borrow().iter() {
        let Some(row) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            continue;
        };
        reorder_row_cells(&row, &order);
    }
}

fn show_shown_columns(columns: &ListColumnLayout, hidden: &[ListColumnId]) {
    for col in ListColumnId::ALL {
        let is_visible = column_shown(hidden, col);
        for widget in columns.cells[col.index()]
            .borrow()
            .iter()
            .filter_map(glib::WeakRef::upgrade)
        {
            widget.set_visible(is_visible);
        }
    }
}

fn reorder_headings(headings: &gtk::Box, order: &[ListColumnId]) {
    let mut heading_children = Vec::new();
    let mut current = headings.first_child();
    while let Some(child) = current {
        current = child.next_sibling();
        heading_children.push(child);
    }
    let mut prev_heading: Option<gtk::Widget> = None;
    for col in order {
        let class = col.css_class();
        if let Some(heading_cell) = heading_children.iter().find(|c| c.has_css_class(class)) {
            headings.reorder_child_after(heading_cell, prev_heading.as_ref());
            prev_heading = Some(heading_cell.clone());
        }
    }
}

fn reorder_row_cells(row: &gtk::Box, order: &[ListColumnId]) {
    let Some(row_struct) = ListRow::from_widget(row.clone()) else {
        return;
    };
    let mut prev_cell: Option<gtk::Widget> = None;
    for col in order {
        let cell = row_cell(&row_struct, *col);
        row.reorder_child_after(&cell, prev_cell.as_ref());
        prev_cell = Some(cell);
    }
}

fn row_cell(row: &ListRow, col: ListColumnId) -> gtk::Widget {
    match col {
        ListColumnId::Name => row.name_cell.clone(),
        ListColumnId::Mode => row.mode.clone().upcast(),
        ListColumnId::Size => row.size.clone().upcast(),
        ListColumnId::Type => row.kind.clone().upcast(),
        ListColumnId::Modified => row.modified.clone().upcast(),
    }
}

impl ModeViews {
    /// Also updates a hidden List pane, which is reused when List is shown again.
    pub fn set_list_columns(&self) {
        let Some((headings, columns, bound_items)) = self.list_table() else {
            return;
        };
        apply_columns_layout(headings, columns, bound_items);
    }

    pub fn open_list_columns_popover(&self) {
        if self.mode != BrowserMode::List {
            return;
        }
        let Some((headings, columns, bound_items)) = self.list_table() else {
            return;
        };
        column_options::open_column_options_popover(headings, headings, columns, bound_items, None);
    }

    fn list_table(&self) -> Option<(&gtk::Box, &ListColumnLayout, &BoundItems)> {
        let pane = self.list_pane.as_ref()?;
        Some((
            pane.list_headings.as_ref()?,
            pane.list_columns.as_ref()?,
            &pane.section.bound_items,
        ))
    }

    pub(in crate::ui) fn cancel_column_drag(&self) -> bool {
        if self.mode != BrowserMode::List {
            return false;
        }
        self.visible_panes().into_iter().any(|pane| {
            let Some(columns) = pane
                .list_columns
                .as_ref()
                .filter(|columns| columns.drag.get().is_some_and(|drag| drag.active))
            else {
                return false;
            };
            let Some(headings) = pane.list_headings.as_ref() else {
                return false;
            };
            // Reset can emit drag-end; discard the pending commit first.
            columns.finish_drag(headings, &pane.section.bound_items, None);
            reset_column_drag(headings);
            true
        })
    }
}

fn reset_column_drag(headings: &gtk::Box) {
    let controllers = headings.observe_controllers();
    if let Some(drag) = (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureDrag>())
    {
        drag.reset();
    }
}
