// SPDX-License-Identifier: MIT

use crate::services::PdfTextLayer;

/// One line of page text: vertical bounds and char range, in rendered pixels.
pub(super) struct PdfLine {
    pub(super) top: f32,
    pub(super) bottom: f32,
    pub(super) start: usize,
    /// Exclusive char index; includes the line's trailing '\n' when present.
    pub(super) end: usize,
}

fn solid(layer: &PdfTextLayer, index: usize) -> Option<[f32; 4]> {
    let rect = layer.glyphs.get(index)?;
    (rect[2] > rect[0] && rect[3] > rect[1]).then_some(*rect)
}

/// Splits `text` at newlines and unions the glyph bounds of each line.
/// Glyphs follow char order, so `glyphs[i]` belongs to the line holding char `i`.
pub(super) fn lines(layer: &PdfTextLayer) -> Vec<PdfLine> {
    let len = len(layer);
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, ch) in layer.text.chars().take(len).enumerate() {
        if ch == '\n' {
            push_line(&mut lines, layer, start, index + 1);
            start = index + 1;
        }
    }
    if start < len {
        push_line(&mut lines, layer, start, len);
    }
    lines
}

fn push_line(lines: &mut Vec<PdfLine>, layer: &PdfTextLayer, start: usize, end: usize) {
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    for index in start..end {
        if let Some(rect) = solid(layer, index) {
            top = top.min(rect[1]);
            bottom = bottom.max(rect[3]);
        }
    }
    if top > bottom {
        // Empty lines carry no ink; inherit height so caret hit-testing can land there.
        let (top, bottom) = lines
            .last()
            .map(|line: &PdfLine| (line.bottom, line.bottom + line.bottom - line.top))
            .unwrap_or((0.0, 0.0));
        lines.push(PdfLine {
            top,
            bottom,
            start,
            end,
        });
        return;
    }
    lines.push(PdfLine {
        top,
        bottom,
        start,
        end,
    });
}

/// The letterboxed image rect of the page inside a widget of `width` x `height`
/// plus the pixel scale, matching gtk::Picture's ContentFit::Contain layout.
pub(super) fn image_bounds(layer: &PdfTextLayer, width: f64, height: f64) -> (f64, f64, f64) {
    let scale = (width / f64::from(layer.width)).min(height / f64::from(layer.height));
    let x = (width - f64::from(layer.width) * scale) / 2.0;
    let y = (height - f64::from(layer.height) * scale) / 2.0;
    (x, y, scale)
}

/// Selectable char count: `glyphs` and `text` are kept in lockstep.
pub(super) fn len(layer: &PdfTextLayer) -> usize {
    layer.text.chars().count().min(layer.glyphs.len())
}

/// Whether the point lands near real glyphs. Presses in the page margins or
/// mid-line gaps beyond the text span keep the pan gesture.
pub(super) fn hit_text(layer: &PdfTextLayer, x: f32, y: f32) -> bool {
    lines(layer).iter().any(|line| {
        let height = (line.bottom - line.top).max(1.0);
        if y < line.top - height * 0.5 || y > line.bottom + height * 0.5 {
            return false;
        }
        let mut x1 = f32::MAX;
        let mut x2 = f32::MIN;
        for index in line.start..line.end {
            if let Some(rect) = solid(layer, index) {
                x1 = x1.min(rect[0]);
                x2 = x2.max(rect[2]);
            }
        }
        x1 <= x2 && x >= x1 - height && x <= x2 + height
    })
}

/// Caret position (a char boundary, 0..=len) nearest the point, in PNG pixels.
/// Horizontal snaps to glyph centers on the nearest line; points off the text
/// snap to the line's start/end, like a document viewer.
pub(super) fn caret_at(layer: &PdfTextLayer, x: f32, y: f32) -> usize {
    let lines = lines(layer);
    let len = len(layer);
    let Some(line) = lines.iter().min_by(|a, b| {
        let da = if y < a.top {
            a.top - y
        } else {
            (y - a.bottom).max(0.0)
        };
        let db = if y < b.top {
            b.top - y
        } else {
            (y - b.bottom).max(0.0)
        };
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return len;
    };
    for index in line.start..line.end {
        let Some(rect) = solid(layer, index) else {
            continue;
        };
        if x < (rect[0] + rect[2]) / 2.0 {
            return index;
        }
    }
    // Past the last glyph: caret sits before the trailing newline, if any.
    let mut end = line.end;
    if end > line.start && layer.text.chars().nth(end - 1) == Some('\n') {
        end -= 1;
    }
    end
}

/// Latin glyphs whose ink crosses the baseline. Runs without them have empty
/// descent space below the baseline, so the band stops early — covering the
/// full font box there reads as a bottom-heavy highlight.
fn has_descender(layer: &PdfTextLayer, start: usize, end: usize) -> bool {
    layer.text.chars().skip(start).take(end - start).any(|ch| {
        matches!(
            ch,
            'g' | 'j' | 'p' | 'q' | 'y' | 'Q' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '_'
        )
    })
}

/// Merged highlight rectangles for the char range `start..end`, in PNG pixels.
/// Runs merge per line so a selected line draws as one block.
pub(super) fn selection_runs(layer: &PdfTextLayer, start: usize, end: usize) -> Vec<[f32; 4]> {
    let (start, end) = (start.min(end), start.max(end));
    lines(layer)
        .iter()
        .filter_map(|line| {
            let (first, last) = (start.max(line.start), end.min(line.end));
            if first >= last {
                return None;
            }
            let mut x1 = f32::MAX;
            let mut x2 = f32::MIN;
            for index in first..last {
                if let Some(rect) = solid(layer, index) {
                    x1 = x1.min(rect[0]);
                    x2 = x2.max(rect[2]);
                }
            }
            // Baseline sits ~4/5 down the font box; runs without descenders
            // stop just past it instead of covering empty descent space.
            let bottom = if has_descender(layer, first, last) {
                line.bottom
            } else {
                line.top + (line.bottom - line.top) * 0.82
            };
            (x1 <= x2).then_some([x1, line.top, x2, bottom])
        })
        .collect()
}

/// The whitespace-delimited word holding `index`; empty when `index` lands on
/// whitespace or at the end of the text.
pub(super) fn word_range(layer: &PdfTextLayer, index: usize) -> (usize, usize) {
    let len = len(layer);
    let index = index.min(len);
    let blank = |i: usize| layer.text.chars().nth(i).is_none_or(char::is_whitespace);
    if blank(index) {
        return (index, index);
    }
    let mut start = index;
    while start > 0 && !blank(start - 1) {
        start -= 1;
    }
    let mut end = index;
    while end < len && !blank(end) {
        end += 1;
    }
    (start, end)
}

/// The visual line holding `index`, excluding its trailing newline.
pub(super) fn line_range(layer: &PdfTextLayer, index: usize) -> (usize, usize) {
    let index = index.min(len(layer));
    let all = lines(layer);
    let Some(line) = all.iter().find(|line| index < line.end).or(all.last()) else {
        return (index, index);
    };
    let mut end = line.end;
    if end > line.start && layer.text.chars().nth(end - 1) == Some('\n') {
        end -= 1;
    }
    (line.start, end)
}

/// The selected text, preserving the page's own newlines.
pub(super) fn selection_text(layer: &PdfTextLayer, start: usize, end: usize) -> String {
    let (start, end) = (start.min(end), start.max(end).min(len(layer)));
    layer.text.chars().skip(start).take(end - start).collect()
}

#[cfg(test)]
mod tests;
