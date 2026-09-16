// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    rc::Rc,
};

use crate::services::DocumentTableCellLayout;
use gtk::{gio, glib, prelude::*};

pub(super) struct TableState {
    rows: Rc<Vec<Vec<DocumentTableCellLayout>>>,
    sort_keys: Rc<Vec<Vec<SortKey>>>,
    header: bool,
    widths: RefCell<Vec<i32>>,
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
            sort: Cell::new(None),
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

    pub(super) fn widget(self: &Rc<Self>) -> gtk::Box {
        let selection = gtk::NoSelection::new(Some(self.model.clone()));
        let view = gtk::ColumnView::new(Some(selection));
        view.add_css_class("preview-document-table");
        view.set_show_column_separators(true);
        view.set_show_row_separators(true);
        let mut columns = Vec::new();
        for index in 0..self.widths.borrow().len() {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let label = gtk::Label::new(None);
                label.add_css_class("preview-document-table-cell");
                label.set_xalign(0.0);
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
                label.set_tooltip_text(None);
                if let Some(cell) = rows[row].get(index) {
                    super::virtual_preview::set_table_cell(&label, cell);
                } else {
                    label.remove_css_class("header");
                    label.set_text("");
                }
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
            column.set_fixed_width(self.widths.borrow()[index]);
            let weak = Rc::downgrade(self);
            column.connect_fixed_width_notify(move |column| {
                if let Some(state) = weak.upgrade() {
                    state.widths.borrow_mut()[index] = column.fixed_width();
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
            sorter.connect_changed(move |sorter, _| {
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
            .max_content_height(400)
            .propagate_natural_height(true)
            .build();
        let copy = gtk::Button::with_label("Copy table");
        copy.add_css_class("preview-code-copy");
        copy.set_halign(gtk::Align::End);
        let weak = Rc::downgrade(self);
        copy.connect_clicked(move |button| {
            if let Some(state) = weak.upgrade() {
                button.clipboard().set_text(&state.copy_text());
            }
        });
        let key = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        let weak_view = view.downgrade();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gtk::gdk::Key::c | gtk::gdk::Key::C)
                && let (Some(state), Some(view)) = (weak.upgrade(), weak_view.upgrade())
            {
                view.clipboard().set_text(&state.copy_text());
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(key);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("preview-table-interactive");
        container.append(&copy);
        container.append(&scroll);
        container
    }
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
