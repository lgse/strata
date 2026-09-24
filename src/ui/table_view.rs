// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    rc::Rc,
};

use crate::services::DocumentTableCellLayout;
use gtk::{gio, glib, prelude::*};

mod selection;

pub(super) struct TableState {
    rows: Rc<Vec<Vec<DocumentTableCellLayout>>>,
    sort_keys: Rc<Vec<Vec<SortKey>>>,
    header: bool,
    widths: RefCell<Vec<i32>>,
    manually_sized: Cell<bool>,
    sort: Cell<Option<(usize, gtk::SortType)>>,
    model: gtk::SortListModel,
}

#[derive(Clone)]
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

impl TableState {
    pub(super) fn new(rows: Vec<Vec<DocumentTableCellLayout>>) -> Rc<Self> {
        let header = rows
            .first()
            .is_some_and(|row| !row.is_empty() && row.iter().all(|cell| cell.header));
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        for index in usize::from(header)..rows.len() {
            store.append(&glib::BoxedAnyObject::new(index));
        }
        // Missing ragged cells share Empty rather than expanding into a dense
        // rows-by-columns matrix outside the parser's value budget.
        let sort_keys = rows
            .iter()
            .map(|row| row.iter().map(|cell| SortKey::new(&cell.text)).collect())
            .collect();
        Rc::new(Self {
            rows: Rc::new(rows),
            sort_keys: Rc::new(sort_keys),
            header,
            widths: RefCell::new(vec![160; columns]),
            manually_sized: Cell::new(false),
            sort: Cell::new((columns > 0).then_some((0, gtk::SortType::Ascending))),
            model: gtk::SortListModel::new(Some(store), None::<gtk::Sorter>),
        })
    }

    pub(super) fn copy_text(&self) -> String {
        let mut text = String::new();
        if self.header {
            append_row(&mut text, &self.rows[0]);
        }
        for position in 0..self.model.n_items() {
            let item = self
                .model
                .item(position)
                .expect("table model position")
                .downcast::<glib::BoxedAnyObject>()
                .expect("table row index");
            append_row(&mut text, &self.rows[*item.borrow::<usize>()]);
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
                label.set_tooltip_text(None);
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
                    .unwrap_or_else(|| format!("Column {}", index + 1))
            } else {
                format!("Column {}", index + 1)
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
            let keys = self.sort_keys.clone();
            let sorter = gtk::CustomSorter::new(move |a, b| {
                let a = *a
                    .downcast_ref::<glib::BoxedAnyObject>()
                    .expect("table sort row index")
                    .borrow::<usize>();
                let b = *b
                    .downcast_ref::<glib::BoxedAnyObject>()
                    .expect("table sort row index")
                    .borrow::<usize>();
                match keys[a]
                    .get(index)
                    .unwrap_or(&SortKey::Empty)
                    .compare(keys[b].get(index).unwrap_or(&SortKey::Empty))
                {
                    Ordering::Less => gtk::Ordering::Smaller,
                    Ordering::Equal => gtk::Ordering::Equal,
                    Ordering::Greater => gtk::Ordering::Larger,
                }
            });
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
        if let Some((index, direction)) = self.sort.get() {
            view.sort_by_column(columns.get(index), direction);
        }
        if let Some(sorter) = view.sorter() {
            self.model.set_sorter(Some(&sorter));
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
                    let primary = sorter.primary_sort_column();
                    let index = weak_columns
                        .iter()
                        .position(|column| column.upgrade() == primary);
                    state
                        .sort
                        .set(index.map(|index| (index, sorter.primary_sort_order())));
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
        view.add_tick_callback(move |view, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let available = (view.width() - 2).max(0);
            if state.manually_sized.get()
                || available == 0
                || measured_width.replace(available) == available
            {
                return glib::ControlFlow::Continue;
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
            glib::ControlFlow::Continue
        });
        let key = gtk::EventControllerKey::new();
        let state = self.clone();
        let weak_view = view.downgrade();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gtk::gdk::Key::c | gtk::gdk::Key::C)
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
