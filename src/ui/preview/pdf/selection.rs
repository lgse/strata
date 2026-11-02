// SPDX-License-Identifier: MIT

use super::*;

/// A point on a page's text layer, in layer units.
pub(super) struct LayerPoint {
    pub(super) page: i32,
    pub(super) layer: Arc<PdfTextLayer>,
    pub(super) x: f32,
    pub(super) y: f32,
}

fn layer_position(
    scroll: &gtk::ScrolledWindow,
    area: &gtk::DrawingArea,
    layer: &PdfTextLayer,
    (x, y): (f64, f64),
) -> Option<(f64, f64)> {
    let point = scroll.compute_point(area, &gtk::graphene::Point::new(x as f32, y as f32))?;
    let (ox, oy, s) =
        pdf_text::image_bounds(layer, f64::from(area.width()), f64::from(area.height()));
    Some((
        (f64::from(point.x()) - ox) / s,
        (f64::from(point.y()) - oy) / s,
    ))
}

fn inside_layer(layer: &PdfTextLayer, px: f64, py: f64) -> bool {
    let inside_x = px >= 0.0 && px <= f64::from(layer.width);
    inside_x && py >= 0.0 && py <= f64::from(layer.height)
}

/// The text under `point`, a scroll coordinate.
pub(super) fn pdf_text_at(
    scroll: &gtk::ScrolledWindow,
    pages: &PdfPages,
    layers: &PdfLayers,
    point: (f64, f64),
) -> Option<LayerPoint> {
    for (page, (_, _, area)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let Some((px, py)) = layer_position(scroll, area, layer, point) else {
            continue;
        };
        if inside_layer(layer, px, py) && pdf_text::hit_text(layer, px as f32, py as f32) {
            return Some(LayerPoint {
                page: *page,
                layer: layer.clone(),
                x: px as f32,
                y: py as f32,
            });
        }
    }
    None
}

/// The closest point on any shown text layer to `point`, a scroll coordinate.
pub(super) fn pdf_text_near(
    scroll: &gtk::ScrolledWindow,
    pages: &PdfPages,
    layers: &PdfLayers,
    point: (f64, f64),
) -> Option<LayerPoint> {
    let mut nearest: Option<(LayerPoint, f64)> = None;
    for (page, (_, _, area)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let Some((px, py)) = layer_position(scroll, area, layer, point) else {
            continue;
        };
        let dx = px.clamp(0.0, f64::from(layer.width)) - px;
        let dy = py.clamp(0.0, f64::from(layer.height)) - py;
        let distance = dx * dx + dy * dy;
        let better = nearest.as_ref().is_none_or(|(_, best)| distance < *best);
        if better {
            nearest = Some((
                LayerPoint {
                    page: *page,
                    layer: layer.clone(),
                    x: (px.clamp(0.0, f64::from(layer.width))) as f32,
                    y: (py.clamp(0.0, f64::from(layer.height))) as f32,
                },
                distance,
            ));
        }
    }
    nearest.map(|(point, _)| point)
}

pub(in crate::ui::preview) fn pdf_desired_ranges(
    layers: &PdfLayers,
    anchor: (i32, usize),
    caret: (i32, usize),
    granularity: u8,
) -> PdfRanges {
    let (anchor_page, anchor) = anchor;
    let (current_page, caret) = caret;
    let snap_lo = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => pdf_text::word_range(layer, index).0,
        3 => pdf_text::line_range(layer, index).0,
        _ => index,
    };
    let snap_hi = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => pdf_text::word_range(layer, index).1,
        3 => pdf_text::line_range(layer, index).1,
        _ => index,
    };
    let (first, last, first_start, last_end) = if anchor_page <= current_page {
        (anchor_page, current_page, anchor, caret)
    } else {
        (current_page, anchor_page, caret, anchor)
    };
    let mut desired = HashMap::new();
    for page in first..=last {
        let Some(layer) = layers.get(&page) else {
            continue;
        };
        let len = pdf_text::len(layer);
        let (start, end) = match (page == first, page == last) {
            (true, true) => (
                snap_lo(layer, first_start.min(last_end)),
                snap_hi(layer, first_start.max(last_end)),
            ),
            (true, false) => (snap_lo(layer, first_start), len),
            (false, true) => (0, snap_hi(layer, last_end)),
            _ => (0, len),
        };
        let (start, end) = (start.min(end), end.min(len));
        if start != end {
            desired.insert(page, (start, end));
        }
    }
    desired
}

pub(in crate::ui::preview) fn pdf_drop_unselected_layer(
    layers: &RefCell<PdfLayers>,
    ranges: &RefCell<PdfRanges>,
    page: i32,
) {
    if !ranges.borrow().contains_key(&page) {
        layers.borrow_mut().remove(&page);
    }
}

pub(in crate::ui::preview) fn pdf_apply_ranges(
    ranges: &RefCell<PdfRanges>,
    layers: &RefCell<PdfLayers>,
    pages: &PdfPages,
    desired: PdfRanges,
) {
    let mut selected = ranges.borrow_mut();
    let mut dirty: Vec<i32> = desired
        .iter()
        .filter(|(page, range)| selected.get(*page) != Some(range))
        .map(|(page, _)| *page)
        .collect();
    dirty.extend(
        selected
            .keys()
            .filter(|page| !desired.contains_key(*page))
            .copied(),
    );
    *selected = desired;
    drop(selected);
    for page in dirty.iter().filter(|page| !pages.contains_key(*page)) {
        pdf_drop_unselected_layer(layers, ranges, *page);
    }
    for (_, _, area) in dirty.iter().filter_map(|page| pages.get(page)) {
        area.queue_draw();
    }
}

pub(in crate::ui::preview) fn pdf_selected_text(layers: &PdfLayers, ranges: &PdfRanges) -> String {
    let mut pages: Vec<_> = ranges.iter().collect();
    pages.sort_by_key(|(page, _)| **page);
    let mut text = String::new();
    for (page, &(start, end)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let part = pdf_text::selection_text(layer, start, end);
        if part.is_empty() {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&part);
    }
    text
}
