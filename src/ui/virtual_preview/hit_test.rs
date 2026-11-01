// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{PreviewUnit, SelectionPoint, VirtualPreviewState};
use crate::services::DocumentSpanStyle;

pub(super) fn point_at(
    list: &gtk::ListView,
    state: &VirtualPreviewState,
    x: f64,
    y: f64,
) -> Option<SelectionPoint> {
    let point = gtk::graphene::Point::new(x as f32, y as f32);
    let bound = state.bound.borrow();
    let mut rows = bound
        .iter()
        .filter_map(|(index, bound)| {
            bound.mapped_root().and_then(|root| {
                root.compute_bounds(list)
                    .map(|bounds| (*index, bound, bounds))
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(index, _, _)| *index);
    let y = y as f32;
    let (index, row, bounds) = rows
        .iter()
        .find(|(_, _, bounds)| bounds.y() <= y && y <= bounds.y() + bounds.height())
        .or_else(|| {
            rows.iter().min_by(|(_, _, left), (_, _, right)| {
                vertical_distance(left, y).total_cmp(&vertical_distance(right, y))
            })
        })?;
    let unit = state.units.get(*index)?;
    if y < bounds.y() || y > bounds.y() + bounds.height() {
        return Some(SelectionPoint {
            unit: *index,
            offset: if y < bounds.y() {
                0
            } else {
                unit.selection_len()
            },
        });
    }
    if row.table.upgrade().is_some() {
        return Some(SelectionPoint {
            unit: *index,
            offset: usize::from(x as f32 >= bounds.x() + bounds.width() / 2.0),
        });
    }
    let Some(view) = row.view.upgrade() else {
        return Some(SelectionPoint {
            unit: *index,
            offset: if y >= bounds.y() + bounds.height() / 2.0 {
                unit.selection_len()
            } else {
                0
            },
        });
    };
    let local = list.compute_point(&view, &point).unwrap_or_else(|| {
        gtk::graphene::Point::new(
            if x as f32 <= bounds.x() {
                0.0
            } else {
                view.width() as f32
            },
            (y - bounds.y()).clamp(0.0, view.height().saturating_sub(1) as f32),
        )
    });
    let (buffer_x, buffer_y) = view.window_to_buffer_coords(
        gtk::TextWindowType::Widget,
        local.x().round() as i32,
        local.y().round() as i32,
    );
    let offset = view
        .iter_at_position(buffer_x, buffer_y)
        .map_or_else(
            || {
                let (mut iter, _) = view.line_at_y(buffer_y);
                if buffer_x > view.iter_location(&iter).x() {
                    view.forward_display_line_end(&mut iter);
                }
                iter.offset() as usize
            },
            |(iter, trailing)| {
                usize::try_from(iter.offset().saturating_add(trailing)).unwrap_or(usize::MAX)
            },
        )
        .min(unit.selection_len());
    Some(SelectionPoint {
        unit: *index,
        offset,
    })
}

pub(super) fn vertical_distance(bounds: &gtk::graphene::Rect, y: f32) -> f32 {
    if y < bounds.y() {
        bounds.y() - y
    } else {
        (y - (bounds.y() + bounds.height())).max(0.0)
    }
}

pub(super) fn link_info_at(
    list: &gtk::ListView,
    state: &VirtualPreviewState,
    x: f64,
    y: f64,
) -> Option<(usize, usize, String)> {
    let point = link_point_at(list, state, x, y)?;
    let PreviewUnit::Document(unit) = state.units.get(point.unit)? else {
        return None;
    };
    unit.spans.iter().enumerate().find_map(|(index, span)| {
        (span.range.start <= point.offset && point.offset < span.range.end)
            .then(|| match &span.style {
                DocumentSpanStyle::Link(uri) => Some((point.unit, index, uri.to_string())),
                _ => None,
            })
            .flatten()
    })
}

pub(super) fn link_point_at(
    list: &gtk::ListView,
    state: &VirtualPreviewState,
    x: f64,
    y: f64,
) -> Option<SelectionPoint> {
    let point = gtk::graphene::Point::new(x as f32, y as f32);
    let bound = state.bound.borrow();
    let (index, row, _) = bound.iter().find_map(|(index, row)| {
        let root = row.mapped_root()?;
        let bounds = root.compute_bounds(list)?;
        (bounds.x() <= point.x()
            && point.x() < bounds.x() + bounds.width()
            && bounds.y() <= point.y()
            && point.y() < bounds.y() + bounds.height())
        .then_some((*index, row, bounds))
    })?;
    let PreviewUnit::Document(unit) = state.units.get(index)? else {
        return None;
    };
    if !unit
        .spans
        .iter()
        .any(|span| matches!(span.style, DocumentSpanStyle::Link(_)))
    {
        return None;
    }
    let view = row.view.upgrade()?;
    let local = list.compute_point(&view, &point)?;
    if local.x() < 0.0
        || local.x() >= view.width() as f32
        || local.y() < 0.0
        || local.y() >= view.height() as f32
    {
        return None;
    }
    let (buffer_x, buffer_y) = view.window_to_buffer_coords(
        gtk::TextWindowType::Widget,
        local.x().round() as i32,
        local.y().round() as i32,
    );
    let (iter, _) = view.iter_at_position(buffer_x, buffer_y)?;
    let glyph = view.iter_location(&iter);
    if buffer_x < glyph.x()
        || buffer_x >= glyph.x() + glyph.width().max(1)
        || buffer_y < glyph.y()
        || buffer_y >= glyph.y() + glyph.height()
    {
        return None;
    }
    Some(SelectionPoint {
        unit: index,
        offset: usize::try_from(iter.offset()).ok()?,
    })
}

pub(super) fn matching_link<'a>(
    pressed: Option<&str>,
    released: Option<&'a str>,
) -> Option<&'a str> {
    released.filter(|released| pressed == Some(*released))
}

pub(super) fn update_link_hover(state: &VirtualPreviewState, next: Option<(usize, usize)>) {
    let previous = state.hovered.replace(next);
    if previous == next {
        return;
    }
    if let Some(previous) = previous {
        set_link_hover(state, previous, false);
    }
    if let Some(next) = next {
        set_link_hover(state, next, true);
    }
}

pub(super) fn set_link_hover(
    state: &VirtualPreviewState,
    (unit_index, span_index): (usize, usize),
    hovered: bool,
) {
    let Some(PreviewUnit::Document(unit)) = state.units.get(unit_index) else {
        return;
    };
    let Some(span) = unit.spans.get(span_index) else {
        return;
    };
    let bound = state.bound.borrow();
    let Some(view) = bound
        .get(&unit_index)
        .and_then(|bound| bound.view.upgrade())
    else {
        return;
    };
    let buffer = view.buffer();
    let Some(tag) = buffer.tag_table().lookup("document-link-hover") else {
        return;
    };
    let start = buffer.iter_at_offset(i32::try_from(span.range.start).unwrap_or(i32::MAX));
    let end = buffer.iter_at_offset(i32::try_from(span.range.end).unwrap_or(i32::MAX));
    if hovered {
        buffer.apply_tag(&tag, &start, &end);
    } else {
        buffer.remove_tag(&tag, &start, &end);
    }
}
