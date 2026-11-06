// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{DocumentSelection, PreviewUnit, VirtualPreviewState};

pub(super) fn update_all_bound_selection(state: &VirtualPreviewState) {
    let indices = state.bound.borrow().keys().copied().collect::<Vec<_>>();
    for index in indices {
        update_bound_selection(state, index);
    }
}

pub(super) fn update_bound_selection(state: &VirtualPreviewState, index: usize) {
    let Some(unit) = state.units.get(index) else {
        return;
    };
    let range = local_selection(state.selection.get(), index, unit.selection_len());
    let bound = state.bound.borrow();
    let Some(bound) = bound.get(&index) else {
        return;
    };
    if let Some(view) = bound.view.upgrade() {
        let buffer = view.buffer();
        let buffer_start = buffer.start_iter();
        let buffer_end = buffer.end_iter();
        let selection_tag = buffer.tag_table().lookup("document-selection");
        if let Some(tag) = selection_tag.as_ref() {
            buffer.remove_tag(tag, &buffer_start, &buffer_end);
        }
        if let Some((start, end)) = range.filter(|(start, end)| start < end) {
            let start = i32::try_from(start)
                .unwrap_or(i32::MAX)
                .min(buffer.char_count());
            let end = i32::try_from(end)
                .unwrap_or(i32::MAX)
                .min(buffer.char_count());
            view.set_selection_range(Some((start, end)));
            if let Some(tag) = selection_tag.as_ref() {
                buffer.apply_tag(
                    tag,
                    &buffer.iter_at_offset(start),
                    &buffer.iter_at_offset(end),
                );
            }
        } else {
            view.set_selection_range(None);
        }
    }
    if let Some(table) = bound.table.upgrade() {
        if range.is_some_and(|(start, end)| start < end) {
            table.add_css_class("selected");
        } else {
            table.remove_css_class("selected");
        }
    }
}

pub(super) fn local_selection(
    selection: Option<DocumentSelection>,
    unit: usize,
    len: usize,
) -> Option<(usize, usize)> {
    let (start, end) = selection?.normalized();
    if start == end || unit < start.unit || unit > end.unit {
        return None;
    }
    let local_start = if unit == start.unit { start.offset } else { 0 };
    let local_end = if unit == end.unit { end.offset } else { len };
    (local_start < local_end).then_some((local_start.min(len), local_end.min(len)))
}

pub(super) fn selection_text(state: &VirtualPreviewState) -> Option<String> {
    let selection = state.selection.get()?;
    if selection.is_empty() {
        return None;
    }
    let (start, end) = selection.normalized();
    let mut output = String::new();
    for index in start.unit..=end.unit {
        let unit = state.units.get(index)?;
        let len = unit.selection_len();
        let from = if index == start.unit {
            start.offset.min(len)
        } else {
            0
        };
        let to = if index == end.unit {
            end.offset.min(len)
        } else {
            len
        };
        if from >= to {
            continue;
        }
        if from == 0 && to == len {
            if let Some(table) = state.tables.borrow().get(&index) {
                output.push_str(&table.copy_text());
            } else {
                output.push_str(unit.copy_text());
            }
        } else {
            match unit {
                PreviewUnit::Document(unit) => output.push_str(&unit.copy_range(from..to)),
                PreviewUnit::Source(_) => {
                    output.push_str(&char_slice(unit.display_text(), from, to))
                }
            }
            if to == len && index < end.unit && unit.copy_text().ends_with('\n') {
                output.push('\n');
            }
        }
    }
    Some(output)
}

pub(super) fn char_slice(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end - start).collect()
}
