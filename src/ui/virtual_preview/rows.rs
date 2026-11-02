// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use super::code_copy::code_copy_button;
use super::highlight::update_bound_selection;
use super::hit_test::set_link_hover;
use super::line_numbers;
use super::source::source_line_numbers;
use super::text_view::{
    bind_document_text_view, bind_inline_math, document_text_view, highlighted_code_language,
    plain_text_view, rendered_width, schedule_document_view_size, set_text_view_content,
};
use super::{BoundRow, PreviewUnit, SourceUnit, VIRTUAL_ROW_MIN_HEIGHT, VirtualPreviewState};
use crate::services::{DocumentUnit, DocumentUnitKind, normalize_preview_text};

pub(super) fn materialize_row(
    row: &gtk::Box,
    index: usize,
    state: &VirtualPreviewState,
    document_tags: Option<&gtk::TextTagTable>,
) {
    if state
        .bound
        .borrow()
        .get(&index)
        .is_some_and(|bound| bound.materialized)
    {
        return;
    }
    let Some(unit) = state.units.get(index) else {
        return;
    };
    let mut bound = if let PreviewUnit::Document(DocumentUnit {
        kind: DocumentUnitKind::Media { source, list_depth },
        text,
        ..
    }) = unit
    {
        clear_box(row);
        row.set_margin_start(16 + list_indent(*list_depth));
        row.set_margin_end(16);
        row.set_margin_top(8);
        row.set_margin_bottom(12);
        state.media_cache.bind((index, None), source, text, row);
        let bound = BoundRow::default();
        bound.root.set(Some(row));
        bound
    } else {
        bind_unit(row, unit, index, state, document_tags)
    };
    align_document_row(row, state.wrapped.get());
    if document_tags.is_some()
        && let Some(view) = bound.view.upgrade()
    {
        if let PreviewUnit::Document(unit) = unit {
            bind_inline_math(&view, unit, index, &state.media_cache);
        }
        schedule_document_view_size(&view, rendered_width(row.width()));
    }
    bound.materialized = true;
    state.bound.borrow_mut().insert(index, bound);
    update_bound_selection(state, index);
    if let Some(hovered) = state.hovered.get().filter(|(unit, _)| *unit == index) {
        set_link_hover(state, hovered, true);
    }
}

pub(super) fn bind_unit(
    row: &gtk::Box,
    unit: &PreviewUnit,
    index: usize,
    state: &VirtualPreviewState,
    document_tags: Option<&gtk::TextTagTable>,
) -> BoundRow {
    let unit_count = state.units.len();
    let units = state.units.clone();
    let wrapped = state.wrapped.get();
    row.set_margin_top(if index == 0 { 12 } else { 0 });
    row.set_margin_bottom(if index + 1 == unit_count { 20 } else { 0 });
    row.set_margin_start(0);
    row.set_margin_end(0);
    row.set_size_request(-1, VIRTUAL_ROW_MIN_HEIGHT);

    match unit {
        PreviewUnit::Document(DocumentUnit {
            kind: DocumentUnitKind::Gap,
            ..
        }) => {
            clear_box(row);
            row.set_size_request(-1, 10);
            let bound = BoundRow::default();
            bound.root.set(Some(row));
            bound
        }
        PreviewUnit::Document(DocumentUnit {
            kind: DocumentUnitKind::Table { rows, list_depth },
            ..
        }) => {
            row.set_margin_start(16 + list_indent(*list_depth));
            row.set_margin_end(16);
            row.set_margin_bottom(if index + 1 == unit_count { 20 } else { 10 });
            clear_box(row);
            let table_state = state
                .tables
                .borrow_mut()
                .entry(index)
                .or_insert_with(|| super::super::table_view::TableState::new(rows.clone()))
                .clone();
            let table = table_state.widget();
            row.append(&table);
            let bound = BoundRow::default();
            bound.root.set(Some(row));
            bound.table.set(Some(&table));
            bound
        }
        PreviewUnit::Document(unit) => {
            row.set_margin_bottom(if index + 1 == unit_count {
                20
            } else if !unit.last {
                0
            } else if document_unit_uses_list_gap(unit) {
                4
            } else {
                10
            });
            let view = bind_document_row(
                row,
                unit,
                units,
                index,
                document_tags.expect("rendered previews should share document text tags"),
                wrapped,
            );
            let bound = BoundRow::default();
            bound.root.set(Some(row));
            bound.view.set(Some(&view));
            bound
        }
        PreviewUnit::Source(unit) => bind_source_row(row, unit, wrapped),
    }
}

pub(super) fn bind_document_row(
    row: &gtk::Box,
    unit: &DocumentUnit,
    units: Rc<Vec<PreviewUnit>>,
    index: usize,
    document_tags: &gtk::TextTagTable,
    wrapped: bool,
) -> super::super::document_view::DocumentTextView {
    let is_code = matches!(unit.kind, DocumentUnitKind::Code { .. });
    let view = if let Some(view) = reusable_document_view(row, is_code, unit) {
        bind_document_text_view(&view, unit, wrapped);
        view
    } else {
        clear_box(row);
        let view = document_text_view(unit, document_tags, wrapped);
        if is_code {
            let overlay = gtk::Overlay::new();
            overlay.add_css_class("preview-code-overlay");
            overlay.set_child(Some(&view));
            row.append(&overlay);
        } else {
            row.append(&view);
        }
        view
    };

    if is_code && let Some(overlay) = row.first_child().and_downcast::<gtk::Overlay>() {
        if let Some(button) = overlay
            .last_child()
            .filter(|child| child.has_css_class("preview-code-copy"))
        {
            overlay.remove_overlay(&button);
        }
        if unit.first {
            overlay.add_overlay(&code_copy_button(units, index));
        }
    }
    view
}

pub(super) fn reusable_document_view(
    row: &gtk::Box,
    is_code: bool,
    unit: &DocumentUnit,
) -> Option<super::super::document_view::DocumentTextView> {
    let child = row.first_child()?;
    let view = if is_code {
        child
            .downcast::<gtk::Overlay>()
            .ok()?
            .child()?
            .downcast::<super::super::document_view::DocumentTextView>()
            .ok()?
    } else {
        child
            .downcast::<super::super::document_view::DocumentTextView>()
            .ok()?
    };
    let highlighted = view.buffer().is::<sourceview5::Buffer>();
    (highlighted == document_uses_source_buffer(unit)).then_some(view)
}

pub(super) fn document_uses_source_buffer(unit: &DocumentUnit) -> bool {
    highlighted_code_language(unit).is_some_and(|language| {
        super::super::theme::source_languages()
            .language(language)
            .is_some()
    })
}

pub(super) fn bind_source_row(row: &gtk::Box, unit: &SourceUnit, wrapped: bool) -> BoundRow {
    let view = if let Some((numbers, view)) = source_row_views(row) {
        numbers.set_text(source_line_numbers(unit));
        let text = normalize_preview_text(unit.display_text());
        set_text_view_content(view.upcast_ref(), &text, view.right_margin());
        view.set_selection_range(None);
        apply_source_view_wrap(&view, &text, wrapped);
        view
    } else {
        clear_box(row);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.add_css_class("preview-source-row");
        let numbers = line_numbers::LineNumbers::new(source_line_numbers(unit));
        let view = plain_text_view(unit.display_text(), true);
        let text = normalize_preview_text(unit.display_text());
        apply_source_view_wrap(&view, &text, wrapped);
        line.append(&numbers);
        line.append(&view);
        row.append(&line);
        view
    };
    let bound = BoundRow::default();
    bound.root.set(Some(row));
    bound.view.set(Some(&view));
    bound
}

pub(super) fn source_row_views(
    row: &gtk::Box,
) -> Option<(
    line_numbers::LineNumbers,
    super::super::document_view::DocumentTextView,
)> {
    let line = row.first_child()?.downcast::<gtk::Box>().ok()?;
    let numbers = line
        .first_child()?
        .downcast::<line_numbers::LineNumbers>()
        .ok()?;
    let view = numbers
        .next_sibling()?
        .downcast::<super::super::document_view::DocumentTextView>()
        .ok()?;
    Some((numbers, view))
}

pub(super) fn align_document_row(row: &gtk::Box, wrapped: bool) {
    if let Some(column) = row
        .layout_manager()
        .and_downcast::<super::super::preview::media_layout::ContentColumn>()
    {
        column.set_start_aligned(!wrapped);
    }
}

pub(super) fn document_wrap_mode(wrapped: bool) -> gtk::WrapMode {
    if wrapped {
        gtk::WrapMode::WordChar
    } else {
        gtk::WrapMode::None
    }
}

pub(super) fn apply_source_view_wrap(
    view: &super::super::document_view::DocumentTextView,
    text: &str,
    wrapped: bool,
) {
    if wrapped {
        view.set_hexpand(true);
        view.set_size_request(-1, -1);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
    } else {
        view.set_hexpand(true);
        view.set_wrap_mode(gtk::WrapMode::None);
        set_text_view_content(view.upcast_ref(), text, view.right_margin());
    }
}

pub(super) fn document_unit_uses_list_gap(unit: &DocumentUnit) -> bool {
    matches!(
        unit.kind,
        DocumentUnitKind::ListItem { .. }
            | DocumentUnitKind::ListChild { .. }
            | DocumentUnitKind::Code {
                list_depth: Some(_),
                ..
            }
            | DocumentUnitKind::Rule {
                list_depth: Some(_)
            }
            | DocumentUnitKind::Table {
                list_depth: Some(_),
                ..
            }
    )
}

pub(super) fn list_indent(depth: Option<usize>) -> i32 {
    depth.map_or(0, |depth| 20 + depth.min(32) as i32 * 18)
}

pub(super) fn clear_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}
