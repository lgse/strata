//! A page's text layer read into lines, and the geometry selection runs on: Strata's
//! `src/ui/preview/pdf_text.rs` (upstream/main e9a8993b7), ported as it is. Offsets are
//! character indices into the layer's text; points are in pixels of the layer's image.

use std::cell::OnceCell;
use std::ops::Range;

use super::model::TextLayer;
use crate::document_view::Folded;

/// One line of the layer: its characters (a line break included) and its ink's top and bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub top: f32,
    pub bottom: f32,
    pub start: usize,
    pub end: usize,
}

/// A text layer, read once: its characters, their boxes, its lines, and (for find) its text
/// folded to lowercase.
#[derive(Debug)]
pub(crate) struct Layer {
    pub width: f32,
    pub height: f32,
    pub text: String,
    chars: Vec<char>,
    /// The byte offset each character starts at, and the text's length last.
    bytes: Vec<usize>,
    glyphs: Vec<[f32; 4]>,
    lines: Vec<Line>,
    folded: OnceCell<Folded>,
}

impl Layer {
    pub fn new(t: TextLayer) -> Layer {
        let chars: Vec<char> = t.text.chars().collect();
        let mut bytes: Vec<usize> = t.text.char_indices().map(|(b, _)| b).collect();
        bytes.push(t.text.len());
        let mut layer = Layer {
            width: t.width.max(1.0),
            height: t.height.max(1.0),
            text: t.text,
            chars,
            bytes,
            glyphs: t.glyphs,
            lines: Vec::new(),
            folded: OnceCell::new(),
        };
        layer.lines = layer.read_lines();
        layer
    }

    /// The characters that have a box: Strata reads only as far as both go.
    pub fn len(&self) -> usize {
        self.chars.len().min(self.glyphs.len())
    }

    /// Whether the layer has any text.
    pub fn has_text(&self) -> bool {
        self.len() > 0
    }

    /// Roughly what the layer holds in memory, for the text budget.
    pub fn bytes(&self) -> usize {
        self.text.len()
            + self.chars.len() * 4
            + self.bytes.len() * 8
            + self.glyphs.len() * 16
            + self.lines.len() * std::mem::size_of::<Line>()
    }

    fn solid(&self, i: usize) -> Option<[f32; 4]> {
        let r = self.glyphs.get(i)?;
        (r[2] > r[0] && r[3] > r[1]).then_some(*r)
    }

    fn read_lines(&self) -> Vec<Line> {
        let len = self.len();
        let mut lines = Vec::new();
        let mut start = 0;
        for i in 0..len {
            if self.chars[i] == '\n' {
                self.push_line(&mut lines, start, i + 1);
                start = i + 1;
            }
        }
        if start < len {
            self.push_line(&mut lines, start, len);
        }
        lines
    }

    fn push_line(&self, lines: &mut Vec<Line>, start: usize, end: usize) {
        let mut top = f32::MAX;
        let mut bottom = f32::MIN;
        for i in start..end {
            if let Some(r) = self.solid(i) {
                top = top.min(r[1]);
                bottom = bottom.max(r[3]);
            }
        }
        if top > bottom {
            // An empty line has no ink: it takes the height of the line above, so a caret can
            // land there.
            let (top, bottom) = lines
                .last()
                .map(|l: &Line| (l.bottom, l.bottom + l.bottom - l.top))
                .unwrap_or((0.0, 0.0));
            lines.push(Line {
                top,
                bottom,
                start,
                end,
            });
            return;
        }
        lines.push(Line {
            top,
            bottom,
            start,
            end,
        });
    }

    #[cfg(test)]
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The horizontal extent of the ink in `range`, if any.
    fn ink_x(&self, range: Range<usize>) -> Option<(f32, f32)> {
        let mut x1 = f32::MAX;
        let mut x2 = f32::MIN;
        for i in range {
            if let Some(r) = self.solid(i) {
                x1 = x1.min(r[0]);
                x2 = x2.max(r[2]);
            }
        }
        (x1 <= x2).then_some((x1, x2))
    }

    /// Whether `(x, y)` lands near text: within half a line above or below a line, and a line
    /// height past either end of its ink. Presses in the margins or past a line's end pan.
    pub fn hit_text(&self, x: f32, y: f32) -> bool {
        self.lines.iter().any(|line| {
            let h = (line.bottom - line.top).max(1.0);
            if y < line.top - h * 0.5 || y > line.bottom + h * 0.5 {
                return false;
            }
            self.ink_x(line.start..line.end)
                .is_some_and(|(x1, x2)| x >= x1 - h && x <= x2 + h)
        })
    }

    /// The caret offset nearest `(x, y)`: on the nearest line, before the first character
    /// whose middle is right of `x`, else at the line's end (before its break).
    pub fn caret_at(&self, x: f32, y: f32) -> usize {
        let dist = |l: &Line| {
            if y < l.top {
                l.top - y
            } else {
                (y - l.bottom).max(0.0)
            }
        };
        let Some(line) = self.lines.iter().min_by(|a, b| {
            dist(a)
                .partial_cmp(&dist(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        }) else {
            return self.len();
        };
        for i in line.start..line.end {
            let Some(r) = self.solid(i) else { continue };
            if x < (r[0] + r[2]) / 2.0 {
                return i;
            }
        }
        self.end_before_break(line)
    }

    fn end_before_break(&self, line: &Line) -> usize {
        let mut end = line.end;
        if end > line.start && self.chars.get(end - 1) == Some(&'\n') {
            end -= 1;
        }
        end
    }

    /// The word around `i`: a run of non-blank characters, or nothing on a blank.
    pub fn word_range(&self, i: usize) -> (usize, usize) {
        let len = self.len();
        let i = i.min(len);
        let blank = |i: usize| self.chars.get(i).is_none_or(|c| c.is_whitespace());
        if blank(i) {
            return (i, i);
        }
        let mut start = i;
        while start > 0 && !blank(start - 1) {
            start -= 1;
        }
        let mut end = i;
        while end < len && !blank(end) {
            end += 1;
        }
        (start, end)
    }

    /// The line around `i`, without its line break.
    pub fn line_range(&self, i: usize) -> (usize, usize) {
        let i = i.min(self.len());
        let Some(line) = self.lines.iter().find(|l| i < l.end).or(self.lines.last()) else {
            return (i, i);
        };
        (line.start, self.end_before_break(line))
    }

    /// Latin glyphs whose ink crosses the baseline. A run without them has empty space below
    /// the baseline, so its band stops early; any non-ASCII character takes the full box
    /// (Strata: missing a descender clips ink, a false one only draws a taller band).
    fn has_descender(&self, start: usize, end: usize) -> bool {
        self.chars[start..end.min(self.chars.len())]
            .iter()
            .any(|&c| {
                !c.is_ascii()
                    || matches!(
                        c,
                        'g' | 'j'
                            | 'p'
                            | 'q'
                            | 'y'
                            | 'Q'
                            | ','
                            | ';'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '_'
                    )
            })
    }

    /// The highlight bands for `start..end`: one per line it touches, `[x1, top, x2, bottom]`,
    /// cut to 82 % of the line's height on runs without descenders.
    pub fn selection_runs(&self, start: usize, end: usize) -> Vec<[f32; 4]> {
        let (start, end) = (start.min(end), start.max(end));
        self.lines
            .iter()
            .filter_map(|line| {
                let (first, last) = (start.max(line.start), end.min(line.end));
                if first >= last {
                    return None;
                }
                let (x1, x2) = self.ink_x(first..last)?;
                let bottom = if self.has_descender(first, last) {
                    line.bottom
                } else {
                    line.top + (line.bottom - line.top) * 0.82
                };
                Some([x1, line.top, x2, bottom])
            })
            .collect()
    }

    /// The text of `start..end`.
    pub fn text_of(&self, start: usize, end: usize) -> String {
        let (start, end) = (start.min(end), start.max(end).min(self.len()));
        self.chars[start..end].iter().collect()
    }

    /// Non-overlapping, case-insensitive matches of `query`, as character ranges.
    pub fn find(&self, query: &str) -> Vec<(usize, usize)> {
        let folded = self.folded.get_or_init(|| Folded::new(&self.text));
        let len = self.len();
        folded
            .find(query)
            .into_iter()
            .map(|r| (self.char_at(r.start), self.char_at(r.end)))
            .filter(|&(s, e)| s < e && e <= len)
            .collect()
    }

    fn char_at(&self, byte: usize) -> usize {
        self.bytes.partition_point(|&b| b < byte)
    }
}
