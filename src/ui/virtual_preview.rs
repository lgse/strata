// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::services::{DocumentLayout, DocumentUnit, DocumentUnitKind, normalize_preview_text};

mod code_copy;
mod highlight;
mod hit_test;
mod keyboard;
mod line_numbers;
mod pointer;
mod rows;
mod scrollbar;
mod selection;
mod source;
mod table_cell;
mod tags;
mod text_view;
#[cfg(test)]
use code_copy::code_block_copy_text;
use highlight::update_all_bound_selection;
#[cfg(test)]
use highlight::{local_selection, selection_text};
use hit_test::point_at;
#[cfg(test)]
use hit_test::{link_point_at, matching_link};
use keyboard::install_keyboard_selection;
#[cfg(test)]
use pointer::drag_threshold_crossed;
use pointer::install_pointer_selection;
pub(super) use pointer::open_web_link;
use rows::{align_document_row, apply_source_view_wrap, document_wrap_mode, materialize_row};
#[cfg(test)]
use rows::{bind_document_row, bind_source_row};
pub(super) use selection::match_ranges;
#[cfg(test)]
use source::source_line_numbers;
use source::source_units;
pub(super) use table_cell::set_table_cell;
#[cfg(test)]
use table_cell::{bounded_text_prefix, styled_markup};
use tags::document_tag_table;
pub(super) use text_view::schedule_document_view_size;
#[cfg(test)]
use text_view::{document_text_view, highlighted_code_language, plain_text_view};
use text_view::{rendered_width, size_document_row};

#[derive(Clone, Copy)]
pub(super) struct TextMatch(DocumentSelection);

const SOURCE_UNIT_BYTES: usize = 16 * 1024;
const SOURCE_UNIT_LINES: usize = 64;
const PATHOLOGICAL_TEXT_UNIT_BYTES: usize = 2 * 1024;
const TABLE_CELL_DISPLAY_BYTES: usize = 512;
const TABLE_CELL_MARKUP_BYTES: usize = 16 * 1024;
const VIRTUAL_ROW_MIN_HEIGHT: i32 = 16;
const AUTOSCROLL_EDGE: f64 = 36.0;

#[derive(Clone)]
enum PreviewUnit {
    Document(DocumentUnit),
    Source(SourceUnit),
}

impl PreviewUnit {
    fn display_text(&self) -> &str {
        match self {
            Self::Document(unit) => &unit.text,
            Self::Source(unit) => unit.display_text(),
        }
    }

    fn copy_text(&self) -> &str {
        match self {
            Self::Document(unit) => &unit.copy_text,
            Self::Source(unit) => &unit.source,
        }
    }

    fn selection_len(&self) -> usize {
        match self {
            Self::Document(DocumentUnit {
                kind: DocumentUnitKind::Table { .. } | DocumentUnitKind::Media { .. },
                ..
            }) => 1,
            _ => self
                .display_text()
                .chars()
                .count()
                .max(usize::from(!self.copy_text().is_empty())),
        }
    }
}

#[derive(Clone)]
struct SourceUnit {
    source: String,
    display_end: usize,
    first_line: usize,
    line_count: usize,
    continuation: bool,
}

impl SourceUnit {
    fn new(source: String, first_line: usize, line_count: usize, continuation: bool) -> Self {
        Self {
            display_end: source.strip_suffix('\n').unwrap_or(&source).len(),
            source,
            first_line,
            line_count,
            continuation,
        }
    }

    fn display_text(&self) -> &str {
        &self.source[..self.display_end]
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SelectionPoint {
    unit: usize,
    offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DocumentSelection {
    anchor: SelectionPoint,
    focus: SelectionPoint,
}

impl DocumentSelection {
    fn normalized(self) -> (SelectionPoint, SelectionPoint) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    fn is_empty(self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Default)]
struct BoundRow {
    materialized: bool,
    root: glib::WeakRef<gtk::Box>,
    view: glib::WeakRef<super::document_view::DocumentTextView>,
    table: glib::WeakRef<gtk::Box>,
}

impl BoundRow {
    fn mapped_root(&self) -> Option<gtk::Box> {
        // GTK keeps prefetched rows bound with stale allocations while hidden.
        self.root.upgrade().filter(|row| row.is_mapped())
    }
}

pub(super) struct VirtualPreviewState {
    units: Rc<Vec<PreviewUnit>>,
    list: glib::WeakRef<gtk::ListView>,
    tables: RefCell<HashMap<usize, Rc<super::table_view::TableState>>>,
    wrapped: Cell<bool>,
    selection: Cell<Option<DocumentSelection>>,
    bound: RefCell<HashMap<usize, BoundRow>>,
    dragging: Cell<bool>,
    press: Cell<(f64, f64)>,
    pointer: Cell<(f64, f64)>,
    drag_generation: Cell<u64>,
    hovered: Cell<Option<(usize, usize)>>,
    pressed_link: RefCell<Option<String>>,
    media_cache: Rc<super::document_media::MediaCache>,
}

pub(super) fn rendered_document(
    layout: DocumentLayout,
    warnings: Vec<String>,
    wrapped: bool,
    document_path: Option<std::path::PathBuf>,
) -> (gtk::Box, Rc<VirtualPreviewState>) {
    let units = layout
        .units
        .into_iter()
        .map(PreviewUnit::Document)
        .collect();
    virtual_preview(units, warnings, false, wrapped, document_path)
}

pub(super) fn source_document(
    content: &str,
    truncated: bool,
    wrapped: bool,
) -> (gtk::Box, Rc<VirtualPreviewState>) {
    let content = normalize_preview_text(content);
    let (source, split_lines) = source_units(&content);
    let units = source.into_iter().map(PreviewUnit::Source).collect();
    let (container, state) = virtual_preview(units, Vec::new(), true, wrapped, None);
    if truncated || split_lines {
        let message = match (truncated, split_lines) {
            (true, true) => {
                "Long source lines are split into virtual rows for responsive scrolling; file reading was limited to the first 1 MB. Copy preserves the complete loaded text and original line breaks."
            }
            (true, false) => "Preview limited to the first 1 MB",
            (false, true) => {
                "Long source lines are split into virtual rows for responsive scrolling. Copy preserves the complete text and original line breaks."
            }
            (false, false) => unreachable!(),
        };
        let notice = gtk::Label::new(Some(&crate::i18n::tr(message)));
        notice.add_css_class("preview-note");
        notice.set_wrap(true);
        container.append(&notice);
    }
    (container, state)
}

pub(super) fn use_virtual_plain_source(content: &str) -> bool {
    content.len() > 128 * 1024
        || content.lines().count() > 512
        || content
            .lines()
            .any(|line| line.len() > PATHOLOGICAL_TEXT_UNIT_BYTES)
}

fn virtual_preview(
    units: Vec<PreviewUnit>,
    warnings: Vec<String>,
    source: bool,
    wrapped: bool,
    document_path: Option<std::path::PathBuf>,
) -> (gtk::Box, Rc<VirtualPreviewState>) {
    let state = Rc::new(VirtualPreviewState {
        units: Rc::new(units),
        list: glib::WeakRef::new(),
        tables: RefCell::new(HashMap::new()),
        wrapped: Cell::new(wrapped),
        selection: Cell::new(None),
        bound: RefCell::new(HashMap::new()),
        dragging: Cell::new(false),
        press: Cell::new((0.0, 0.0)),
        pointer: Cell::new((0.0, 0.0)),
        drag_generation: Cell::new(0),
        hovered: Cell::new(None),
        pressed_link: RefCell::new(None),
        media_cache: super::document_media::MediaCache::new(document_path),
    });
    if let [
        PreviewUnit::Document(DocumentUnit {
            kind: DocumentUnitKind::Table { rows, .. },
            ..
        }),
    ] = state.units.as_slice()
    {
        let table = super::table_view::TableState::new(rows.clone());
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_vexpand(true);
        for warning in warnings {
            container.append(&super::preview::document_notice(&warning));
        }
        container.append(&table.widget_with_height(true));
        state.tables.borrow_mut().insert(0, table);
        return (container, state);
    }
    let model = gtk::StringList::new(&vec![""; state.units.len()]);
    let selection = gtk::NoSelection::new(Some(model.clone()));
    let document_tags = (!source).then(document_tag_table);
    let factory = gtk::SignalListItemFactory::new();
    let state_for_map = Rc::downgrade(&state);
    let document_tags_for_map = document_tags.clone();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
        row.add_css_class("preview-virtual-row");
        row.set_size_request(-1, VIRTUAL_ROW_MIN_HEIGHT);
        row.set_hexpand(true);
        if !source {
            row.set_layout_manager(Some(super::preview::media_layout::ContentColumn::new()));
            let weak_item = item.downgrade();
            let state_for_map = state_for_map.clone();
            let document_tags = document_tags_for_map.clone();
            row.connect_map(move |row| {
                if let (Some(item), Some(state)) = (weak_item.upgrade(), state_for_map.upgrade()) {
                    materialize_row(
                        row,
                        item.position() as usize,
                        &state,
                        document_tags.as_ref(),
                    );
                }
            });
        }
        item.set_child(Some(&row));
    });

    let state_for_bind = state.clone();
    let document_tags_for_bind = document_tags.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let index = item.position() as usize;
        let bound = BoundRow::default();
        bound.root.set(Some(&row));
        state_for_bind.bound.borrow_mut().insert(index, bound);
        if source {
            materialize_row(&row, index, &state_for_bind, None);
        } else if row.is_mapped() {
            // GTK rebinds rows before unmapping its old prefetch window. Discard
            // intermediate bindings before building their widget trees.
            let weak_state = Rc::downgrade(&state_for_bind);
            let weak_item = item.downgrade();
            let document_tags = document_tags_for_bind.clone();
            row.add_tick_callback(move |row, _| {
                if let (Some(state), Some(item)) = (weak_state.upgrade(), weak_item.upgrade())
                    && row.is_mapped()
                    && item.position() as usize == index
                {
                    materialize_row(row, index, &state, document_tags.as_ref());
                }
                glib::ControlFlow::Break
            });
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

    let list = gtk::ListView::new(Some(selection), Some(factory));
    list.add_css_class("preview-virtual-list");
    list.add_css_class(if source {
        "preview-virtual-source"
    } else {
        "preview-document"
    });
    list.set_accessible_role(gtk::AccessibleRole::Document);
    list.set_focusable(true);
    list.set_hexpand(true);
    list.set_vexpand(true);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();
    if !source {
        let weak_state = Rc::downgrade(&state);
        let measured_width = Cell::new(0);
        scroll
            .hadjustment()
            .connect_page_size_notify(move |adjustment| {
                let width = adjustment.page_size().round() as i32;
                if width <= 0 || measured_width.replace(width) == width {
                    return;
                }
                let Some(state) = weak_state.upgrade() else {
                    return;
                };
                for bound in state.bound.borrow().values() {
                    size_document_row(bound, rendered_width(width));
                }
            });
    }
    state.list.set(Some(&list));
    install_pointer_selection(&list, &scroll, state.clone());
    install_keyboard_selection(&list, state.clone());

    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_vexpand(true);
    for warning in warnings {
        container.append(&super::preview::document_notice(&warning));
    }
    container.append(&scrollbar::wrap(&scroll, &state));
    (container, state)
}

impl VirtualPreviewState {
    pub(super) fn can_find(&self) -> bool {
        self.units
            .iter()
            .any(|unit| !unit.display_text().is_empty())
    }

    pub(super) fn find_matches(&self, query: &str) -> Vec<TextMatch> {
        let mut text = String::new();
        let mut starts = Vec::new();
        let mut length = 0;
        for (index, unit) in self.units.iter().enumerate() {
            if unit.display_text().is_empty() {
                continue;
            }
            starts.push((length, index));
            text.push_str(unit.display_text());
            length += unit.display_text().chars().count();
            if unit.copy_text().ends_with('\n') && !unit.display_text().ends_with('\n') {
                text.push('\n');
                length += 1;
            }
        }
        if starts.is_empty() {
            return Vec::new();
        }
        let point = |offset: usize| {
            let (start, unit) = starts[starts
                .partition_point(|(start, _)| *start <= offset)
                .saturating_sub(1)];
            SelectionPoint {
                unit,
                offset: (offset - start).min(self.units[unit].selection_len()),
            }
        };
        match_ranges(&text, query)
            .into_iter()
            .map(|(start, end)| {
                TextMatch(DocumentSelection {
                    anchor: point(start),
                    focus: point(end),
                })
            })
            .collect()
    }

    pub(super) fn select_match(self: &Rc<Self>, found: TextMatch) {
        self.select_and_reveal(found.0);
    }

    pub(super) fn clear_selection(&self) {
        self.selection.set(None);
        update_all_bound_selection(self);
    }

    fn select_and_reveal(self: &Rc<Self>, selection: DocumentSelection) {
        self.selection.set(Some(selection));
        update_all_bound_selection(self);
        let Some(list) = self.list.upgrade() else {
            return;
        };
        list.scroll_to(
            selection.focus.unit as u32,
            gtk::ListScrollFlags::NONE,
            None,
        );
        let state = Rc::downgrade(self);
        let frames = Cell::new(0);
        list.add_tick_callback(move |list, _| {
            let Some(state) = state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            frames.set(frames.get() + 1);
            if state.selection.get() != Some(selection) || frames.get() > 30 {
                return glib::ControlFlow::Break;
            }
            let view = state
                .bound
                .borrow()
                .get(&selection.focus.unit)
                .and_then(|row| row.view.upgrade());
            let Some(view) = view else {
                return glib::ControlFlow::Continue;
            };
            let Some(scroll) = list
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()
            else {
                return glib::ControlFlow::Break;
            };
            let Some(bounds) = view.compute_bounds(&scroll) else {
                return glib::ControlFlow::Continue;
            };
            let iter = view.buffer().iter_at_offset(
                selection
                    .focus
                    .offset
                    .min(view.buffer().char_count() as usize) as i32,
            );
            let rect = view.iter_location(&iter);
            let (_, y) =
                view.buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
            let adjustment = scroll.vadjustment();
            let top = f64::from(bounds.y()) + f64::from(y);
            if top < 0.0 || top + f64::from(rect.height()) > adjustment.page_size() {
                let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
                adjustment.set_value(
                    (adjustment.value() + top - adjustment.page_size() / 2.0)
                        .clamp(adjustment.lower(), maximum),
                );
            }
            glib::ControlFlow::Break
        });
    }

    pub(super) fn set_wrapped(&self, wrapped: bool) {
        if self.wrapped.replace(wrapped) == wrapped {
            return;
        }
        for (index, bound) in self.bound.borrow().iter() {
            let Some(view) = bound.view.upgrade() else {
                continue;
            };
            match self.units.get(*index) {
                Some(PreviewUnit::Document(_)) => {
                    view.set_wrap_mode(document_wrap_mode(wrapped));
                    if wrapped {
                        view.set_width_request(-1);
                    }
                    // GTK reports the previous mode's height until the view has
                    // been allocated and revalidated in the new one.
                    view.add_tick_callback(|view, _| {
                        schedule_document_view_size(view, view.width());
                        glib::ControlFlow::Break
                    });
                }
                Some(PreviewUnit::Source(unit)) => {
                    apply_source_view_wrap(
                        &view,
                        &normalize_preview_text(unit.display_text()),
                        wrapped,
                    );
                }
                None => {}
            }
            if let Some(row) = bound.root.upgrade() {
                align_document_row(&row, wrapped);
                row.queue_resize();
            }
        }
    }
}

#[cfg(test)]
mod tests;
