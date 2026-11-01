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
    let (index, row, bounds) = row_nearest(&rows, point.y())?;
    let unit = state.units.get(*index)?;
    let at = |offset| {
        Some(SelectionPoint {
            unit: *index,
            offset,
        })
    };
    if point.y() < bounds.y() || point.y() > bounds.y() + bounds.height() {
        return at(if point.y() < bounds.y() {
            0
        } else {
            unit.selection_len()
        });
    }
    if row.table.upgrade().is_some() {
        return at(usize::from(point.x() >= bounds.x() + bounds.width() / 2.0));
    }
    let Some(view) = row.view.upgrade() else {
        return at(if point.y() >= bounds.y() + bounds.height() / 2.0 {
            unit.selection_len()
        } else {
            0
        });
    };
    at(text_offset_at(list, &view, &point, bounds).min(unit.selection_len()))
}

/// The row under `y`, or the closest one when `y` falls between rows.
fn row_nearest<'a, T>(
    rows: &'a [(usize, T, gtk::graphene::Rect)],
    y: f32,
) -> Option<&'a (usize, T, gtk::graphene::Rect)> {
    rows.iter()
        .find(|(_, _, bounds)| bounds.y() <= y && y <= bounds.y() + bounds.height())
        .or_else(|| {
            rows.iter().min_by(|(_, _, left), (_, _, right)| {
                vertical_distance(left, y).total_cmp(&vertical_distance(right, y))
            })
        })
}

/// The character offset nearest `point`, a list coordinate inside the row `bounds`.
fn text_offset_at(
    list: &gtk::ListView,
    view: &super::super::document_view::DocumentTextView,
    point: &gtk::graphene::Point,
    bounds: &gtk::graphene::Rect,
) -> usize {
    let local = list.compute_point(view, point).unwrap_or_else(|| {
        gtk::graphene::Point::new(
            if point.x() <= bounds.x() {
                0.0
            } else {
                view.width() as f32
            },
            (point.y() - bounds.y()).clamp(0.0, view.height().saturating_sub(1) as f32),
        )
    });
    let (buffer_x, buffer_y) = view.window_to_buffer_coords(
        gtk::TextWindowType::Widget,
        local.x().round() as i32,
        local.y().round() as i32,
    );
    view.iter_at_position(buffer_x, buffer_y).map_or_else(
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
    let (index, row) = bound.iter().find_map(|(index, row)| {
        let bounds = row.mapped_root()?.compute_bounds(list)?;
        rect_contains(&bounds, &point).then_some((*index, row))
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
    if !within_widget(&local, &view) {
        return None;
    }
    let (buffer_x, buffer_y) = view.window_to_buffer_coords(
        gtk::TextWindowType::Widget,
        local.x().round() as i32,
        local.y().round() as i32,
    );
    let (iter, _) = view.iter_at_position(buffer_x, buffer_y)?;
    if !glyph_contains(&view.iter_location(&iter), buffer_x, buffer_y) {
        return None;
    }
    Some(SelectionPoint {
        unit: index,
        offset: usize::try_from(iter.offset()).ok()?,
    })
}

/// Half-open on the far edges, unlike `graphene::Rect::contains_point`.
fn rect_contains(rect: &gtk::graphene::Rect, point: &gtk::graphene::Point) -> bool {
    let inside_x = rect.x() <= point.x() && point.x() < rect.x() + rect.width();
    inside_x && rect.y() <= point.y() && point.y() < rect.y() + rect.height()
}

fn within_widget(local: &gtk::graphene::Point, widget: &impl IsA<gtk::Widget>) -> bool {
    let inside_x = local.x() >= 0.0 && local.x() < widget.width() as f32;
    inside_x && local.y() >= 0.0 && local.y() < widget.height() as f32
}

fn glyph_contains(glyph: &gtk::gdk::Rectangle, x: i32, y: i32) -> bool {
    let inside_x = x >= glyph.x() && x < glyph.x() + glyph.width().max(1);
    inside_x && y >= glyph.y() && y < glyph.y() + glyph.height()
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
