// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, OnceCell, RefCell},
    cmp::Ordering,
    rc::Rc,
};

use crate::services::DocumentTableCellLayout;
use crate::ui::keys::{self, bindings::COPY};
use gtk::{glib, prelude::*};

mod model;
mod selection;

use model::RowModel;

pub(super) struct TableState {
    rows: Rc<Vec<Vec<DocumentTableCellLayout>>>,
    column_sort_keys: Vec<OnceCell<ColumnSortKeys>>,
    column_row_counts: Vec<usize>,
    header: bool,
    widths: RefCell<Vec<i32>>,
    manually_sized: Cell<bool>,
    sort: RefCell<Vec<(usize, gtk::SortType)>>,
    model: RowModel,
}

enum SortKey {
    Number(f64),
    Text(String),
    Empty,
}

impl SortKey {
    fn new(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() {
            return Self::Empty;
        }
        match text.parse::<f64>() {
            Ok(number) if number.is_finite() => Self::Number(number),
            _ => Self::Text(text.to_lowercase()),
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Empty, Self::Empty) => Ordering::Equal,
            (Self::Number(_), _) | (Self::Text(_), Self::Empty) => Ordering::Less,
            _ => Ordering::Greater,
        }
    }
}

enum ColumnSortKeys {
    Dense(Vec<SortKey>),
    Sparse(Vec<(usize, SortKey)>),
}

impl ColumnSortKeys {
    fn new(rows: &[Vec<DocumentTableCellLayout>], column: usize, present: usize) -> Self {
        // Dense indexing is faster, but may reserve at most twice the real
        // cells. Sparse columns never expand to the table's full row count.
        if present >= rows.len().div_ceil(2) {
            Self::Dense(
                rows.iter()
                    .map(|row| {
                        row.get(column)
                            .map_or(SortKey::Empty, |cell| SortKey::new(&cell.text))
                    })
                    .collect(),
            )
        } else {
            let mut keys = Vec::with_capacity(present);
            keys.extend(rows.iter().enumerate().filter_map(|(row, cells)| {
                cells
                    .get(column)
                    .map(|cell| (row, SortKey::new(&cell.text)))
            }));
            Self::Sparse(keys)
        }
    }

    fn compare(&self, a: usize, b: usize) -> Ordering {
        match self {
            Self::Dense(keys) => keys[a].compare(&keys[b]),
            Self::Sparse(keys) => {
                let key = |row| {
                    keys.binary_search_by_key(&row, |(row, _)| *row)
                        .map_or(&SortKey::Empty, |index| &keys[index].1)
                };
                key(a).compare(key(b))
            }
        }
    }
}

impl TableState {
    pub(super) fn new(rows: Vec<Vec<DocumentTableCellLayout>>) -> Rc<Self> {
        let header = rows
            .first()
            .is_some_and(|row| !row.is_empty() && row.iter().all(|cell| cell.header));
        let mut column_row_counts = Vec::new();
        for row in &rows {
            if column_row_counts.len() <= row.len() {
                column_row_counts.resize(row.len() + 1, 0);
            }
            column_row_counts[row.len()] += 1;
        }
        if header {
            column_row_counts[rows[0].len()] -= 1;
        }
        let columns = column_row_counts.len().saturating_sub(1);
        let mut present = 0;
        // A row-width histogram gives every column's population in O(rows + columns).
        for count in column_row_counts.iter_mut().rev() {
            let at_width = *count;
            *count = present;
            present += at_width;
        }
        column_row_counts.truncate(columns);
        let model = RowModel::new(rows.len());
        let state = Rc::new(Self {
            rows: Rc::new(rows),
            column_sort_keys: (0..columns).map(|_| OnceCell::new()).collect(),
            column_row_counts,
            header,
            widths: RefCell::new(vec![160; columns]),
            manually_sized: Cell::new(false),
            sort: RefCell::new(Vec::new()),
            model,
        });
        if columns > 0 {
            state.apply_sort(&[(0, gtk::SortType::Ascending)]);
        } else {
            state.restore_natural_order();
        }
        state
    }

    fn sort_keys_for_column(&self, column: usize) -> &ColumnSortKeys {
        self.column_sort_keys[column].get_or_init(|| {
            ColumnSortKeys::new(
                &self.rows[usize::from(self.header)..],
                column,
                self.column_row_counts[column],
            )
        })
    }

    fn apply_sort(&self, columns: &[(usize, gtk::SortType)]) {
        let start = usize::from(self.header);
        if self.sort.borrow().as_slice() == columns
            && self.model.order().len() == self.rows.len() - start
        {
            return;
        }
        let previous_sort = self.sort.replace(columns.to_vec());
        if columns.is_empty() {
            self.restore_natural_order();
            return;
        }
        let keys = columns
            .iter()
            .map(|&(column, direction)| (self.sort_keys_for_column(column), direction))
            .collect::<Vec<_>>();
        if previous_sort.len() == columns.len()
            && previous_sort[0].0 == columns[0].0
            && previous_sort[0].1 != columns[0].1
            && previous_sort[1..] == columns[1..]
        {
            let mut order = self.model.order().to_vec();
            order.reverse();
            // Reverse primary-key groups, not their stable secondary ordering.
            for group in order
                .chunk_by_mut(|&a, &b| keys[0].0.compare(a - start, b - start) == Ordering::Equal)
            {
                group.reverse();
            }
            self.model.set_order(order);
            return;
        }
        let mut order = (0..self.rows.len() - start).collect::<Vec<_>>();
        if let [(ColumnSortKeys::Dense(keys), direction)] = keys.as_slice() {
            if *direction == gtk::SortType::Descending {
                order.sort_by(|&a, &b| keys[b].compare(&keys[a]));
            } else {
                order.sort_by(|&a, &b| keys[a].compare(&keys[b]));
            }
        } else {
            order.sort_by(|&a, &b| {
                for &(keys, direction) in &keys {
                    let cmp = keys.compare(a, b);
                    if cmp != Ordering::Equal {
                        return if direction == gtk::SortType::Descending {
                            cmp.reverse()
                        } else {
                            cmp
                        };
                    }
                }
                Ordering::Equal
            });
        }
        if start > 0 {
            for row in &mut order {
                *row += start;
            }
        }
        self.model.set_order(order);
    }

    fn restore_natural_order(&self) {
        self.model
            .set_order((usize::from(self.header)..self.rows.len()).collect());
    }

    pub(super) fn copy_text(&self) -> String {
        let mut text = String::new();
        if self.header {
            append_row(&mut text, &self.rows[0]);
        }
        for &row in self.model.order().iter() {
            append_row(&mut text, &self.rows[row]);
        }
        text
    }

    fn measure_columns(&self, view: &gtk::ColumnView) -> Vec<i32> {
        let layout = view.create_pango_layout(None);
        let mut widths = vec![72; self.widths.borrow().len()];
        for (row_index, row) in self.rows.iter().take(64).enumerate() {
            for (index, cell) in row.iter().enumerate() {
                for line in cell.text.lines().take(8) {
                    layout.set_text(&line.chars().take(128).collect::<String>());
                    let padding = if row_index == 0 && self.header {
                        40
                    } else {
                        36
                    };
                    widths[index] = widths[index].max((layout.pixel_size().0 + padding).min(640));
                }
            }
        }
        widths
    }

    pub(super) fn widget(self: &Rc<Self>) -> gtk::Box {
        self.widget_with_height(false)
    }

    pub(super) fn widget_with_height(self: &Rc<Self>, fill_height: bool) -> gtk::Box {
        let selection = gtk::NoSelection::new(Some(self.model.clone()));
        let view = gtk::ColumnView::new(Some(selection));
        view.add_css_class("preview-document-table");
        // GTK clears the header cursor after checking divider hit targets. Its
        // fallback must come from the column view, not the outer text renderer.
        view.set_cursor_from_name(Some("pointer"));
        if let Some(body) = view
            .last_child()
            .filter(|child| child.css_name() == "listview")
        {
            body.set_cursor_from_name(Some("default"));
        }
        view.set_reorderable(false);
        view.set_show_column_separators(true);
        view.set_show_row_separators(true);
        let desired_widths = self.measure_columns(&view);
        let automatic_resize = Rc::new(Cell::new(false));
        let text_selection = selection::Selection::new(self.clone());
        let mut columns = Vec::new();
        for (index, desired_width) in desired_widths.iter().enumerate() {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let label = gtk::Label::new(None);
                label.add_css_class("preview-document-table-cell");
                label.set_xalign(0.0);
                label.set_selectable(true);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.connect_activate_link(|label, uri| {
                    if crate::services::has_web_scheme(uri) {
                        super::virtual_preview::open_web_link(uri, label);
                    }
                    glib::Propagation::Stop
                });
                item.set_child(Some(&label));
            });
            let rows = self.rows.clone();
            let selection_for_bind = text_selection.clone();
            factory.connect_bind(move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                    return;
                };
                let Some(label) = item.child().and_downcast::<gtk::Label>() else {
                    return;
                };
                let row = *object.borrow::<usize>();
                label.select_region(0, 0);
                crate::ui::accessibility::set_description(&label, None);
                if let Some(cell) = rows[row].get(index) {
                    super::virtual_preview::set_table_cell(&label, cell);
                } else {
                    label.remove_css_class("header");
                    label.set_text("");
                }
                selection_for_bind.bind(&label, item, index);
            });
            let title = if self.header {
                self.rows[0]
                    .get(index)
                    .map(|cell| cell.text.chars().take(128).collect::<String>())
                    .unwrap_or_else(|| {
                        rust_i18n::t!("Column %{value1}", value1 = index + 1).into_owned()
                    })
            } else {
                rust_i18n::t!("Column %{value1}", value1 = index + 1).into_owned()
            };
            let column = gtk::ColumnViewColumn::new(Some(&title), Some(factory));
            column.set_resizable(true);
            column.set_fixed_width(if self.manually_sized.get() {
                self.widths.borrow()[index]
            } else {
                *desired_width
            });
            let weak = Rc::downgrade(self);
            let automatic_resize = automatic_resize.clone();
            column.connect_fixed_width_notify(move |column| {
                if let Some(state) = weak.upgrade() {
                    state.widths.borrow_mut()[index] = column.fixed_width();
                    if !automatic_resize.get() {
                        state.manually_sized.set(true);
                    }
                }
            });
            let sorter = gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal);
            column.set_sorter(Some(&sorter));
            view.append_column(&column);
            columns.push(column);
        }
        if let Some(header) = view.first_child() {
            let mut title = header.first_child();
            while let Some(widget) = title {
                title = widget.next_sibling();
                if let Some(content) = widget.first_child().and_downcast::<gtk::Box>() {
                    if let Some(label) = content.first_child().and_downcast::<gtk::Label>() {
                        label.set_hexpand(true);
                        label.set_xalign(0.0);
                    }
                    if let Some(indicator) = content
                        .last_child()
                        .filter(|child| child.css_name() == "sort-indicator")
                    {
                        indicator.set_halign(gtk::Align::End);
                        indicator.set_valign(gtk::Align::Center);
                        indicator.set_size_request(12, 12);
                    }
                }
            }
        }
        if let (Some(header), Some(column)) = (view.first_child(), columns.last())
            && let Some(title) = header.last_child()
        {
            install_last_column_resize(&title, column);
        }
        if let Some(&(index, direction)) = self.sort.borrow().first() {
            view.sort_by_column(columns.get(index), direction);
        }
        if let Some(sorter) = view.sorter() {
            let weak = Rc::downgrade(self);
            let weak_columns = columns
                .iter()
                .map(|column| column.downgrade())
                .collect::<Vec<_>>();
            let weak_view = view.downgrade();
            let selection_for_sort = Rc::downgrade(&text_selection);
            sorter.connect_changed(move |sorter, _| {
                if let Some(selection) = selection_for_sort.upgrade() {
                    selection.clear();
                }
                let Some(view) = weak_view.upgrade() else {
                    return;
                };
                if view.columns().n_items() as usize != weak_columns.len() {
                    return;
                }
                if let Some(state) = weak.upgrade()
                    && let Some(sorter) = sorter.downcast_ref::<gtk::ColumnViewSorter>()
                {
                    let columns = (0..sorter.n_sort_columns())
                        .filter_map(|position| {
                            let (column, direction) = sorter.nth_sort_column(position);
                            let column = column?;
                            weak_columns
                                .iter()
                                .position(|weak| weak.upgrade().as_ref() == Some(&column))
                                .map(|index| (index, direction))
                        })
                        .collect::<Vec<_>>();
                    state.apply_sort(&columns);
                    if state.model.n_items() > 0 {
                        let weak_view = view.downgrade();
                        glib::idle_add_local_once(move || {
                            if let Some(view) = weak_view.upgrade() {
                                view.scroll_to(0, None, gtk::ListScrollFlags::NONE, None);
                            }
                        });
                    }
                }
            });
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height((self.rows.len().min(10) as i32 * 40 + 32).clamp(120, 400))
            .max_content_height(if fill_height { -1 } else { 400 })
            .propagate_natural_height(!fill_height)
            .vexpand(fill_height)
            .build();
        text_selection.install(&view, &scroll);
        let weak = Rc::downgrade(self);
        let weak_columns = columns
            .iter()
            .map(|column| column.downgrade())
            .collect::<Vec<_>>();
        let measured_width = Cell::new(0);
        let resize = Rc::new(move |view: &gtk::ColumnView| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let available = (view.width() - 2).max(0);
            if state.manually_sized.get()
                || available == 0
                || measured_width.replace(available) == available
            {
                return;
            }
            let minimum_total = desired_widths
                .iter()
                .map(|width| (*width).min(160))
                .sum::<i32>();
            let extra = (available - minimum_total).max(0);
            let weights = desired_widths
                .iter()
                .map(|width| (width - (*width).min(160)).max(1))
                .sum::<i32>()
                .max(1);
            let mut remaining = extra;
            automatic_resize.set(true);
            for (index, weak_column) in weak_columns.iter().enumerate() {
                let share = if index + 1 == weak_columns.len() {
                    remaining
                } else {
                    (i64::from(extra)
                        * i64::from(
                            (desired_widths[index] - desired_widths[index].min(160)).max(1),
                        )
                        / i64::from(weights)) as i32
                };
                remaining -= share;
                if let Some(column) = weak_column.upgrade() {
                    column.set_fixed_width(desired_widths[index].min(160) + share);
                }
            }
            automatic_resize.set(false);
        });
        let weak_view = view.downgrade();
        let resize_pending = Rc::new(Cell::new(false));
        scroll.hadjustment().connect_page_size_notify(move |_| {
            let Some(view) = weak_view.upgrade() else {
                return;
            };
            if resize_pending.replace(true) {
                return;
            }
            let resize = resize.clone();
            let resize_pending = resize_pending.clone();
            // Allocation and column notifications must settle before fitting.
            view.add_tick_callback(move |view, _| {
                resize(view);
                resize_pending.set(false);
                glib::ControlFlow::Break
            });
        });
        let key = gtk::EventControllerKey::new();
        let state = self.clone();
        let weak_view = view.downgrade();
        keys::on_press(&key, move |_, press| {
            if press.is(COPY)
                && let Some(view) = weak_view.upgrade()
            {
                view.clipboard().set_text(&state.copy_text());
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(key);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("preview-table-interactive");
        container.set_vexpand(fill_height);
        container.append(&scroll);
        container
    }
}

fn install_last_column_resize(title: &gtk::Widget, column: &gtk::ColumnViewColumn) {
    // GTK intentionally excludes the final column from its divider hit test.
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let active = Rc::new(Cell::new(false));
    let start_width = Rc::new(Cell::new(0));
    let active_begin = active.clone();
    let width_begin = start_width.clone();
    let weak_title = title.downgrade();
    let weak_column = column.downgrade();
    drag.connect_drag_begin(move |gesture, x, _| {
        let (Some(title), Some(column)) = (weak_title.upgrade(), weak_column.upgrade()) else {
            return;
        };
        let resizing = x >= f64::from(title.width() - 8);
        active_begin.set(resizing);
        if resizing {
            width_begin.set(column.fixed_width());
            gesture.set_state(gtk::EventSequenceState::Claimed);
        } else {
            gesture.set_state(gtk::EventSequenceState::Denied);
        }
    });
    let weak_column = column.downgrade();
    drag.connect_drag_update(move |_, offset, _| {
        if active.get()
            && let Some(column) = weak_column.upgrade()
        {
            column.set_fixed_width((start_width.get() + offset.round() as i32).max(48));
        }
    });
    title.add_controller(drag);
    let motion = gtk::EventControllerMotion::new();
    let weak_title = title.downgrade();
    motion.connect_motion(move |_, x, _| {
        if let Some(title) = weak_title.upgrade() {
            title.set_cursor_from_name(Some(if x >= f64::from(title.width() - 8) {
                "col-resize"
            } else {
                "pointer"
            }));
        }
    });
    let weak_title = title.downgrade();
    motion.connect_leave(move |_| {
        if let Some(title) = weak_title.upgrade() {
            title.set_cursor(None);
        }
    });
    title.add_controller(motion);
}

fn append_row(text: &mut String, row: &[DocumentTableCellLayout]) {
    for (index, cell) in row.iter().enumerate() {
        if index > 0 {
            text.push('\t');
        }
        text.push_str(&cell.text);
    }
    text.push('\n');
}

#[cfg(test)]
mod tests;
