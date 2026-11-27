// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::model::Shared;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Unit {
    #[default]
    Char,
    Word,
    Line,
}

impl Unit {
    /// The unit a press's click count selects by: 1, 2, then 3 and beyond.
    pub fn of_clicks(clicks: u32) -> Unit {
        match clicks {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        }
    }
}

/// Each page's selected range `(start, end)`, start before end, never empty.
pub(crate) type Ranges = BTreeMap<usize, (usize, usize)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Selection {
    /// Where the selection started: a page and a character. `None` after a pan or a clear.
    pub anchor: Option<(usize, usize)>,
    pub unit: Unit,
    pub ranges: Ranges,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Selection::default();
    }

    pub fn press(&mut self, layer: &Shared, page: usize, caret: usize, unit: Unit) {
        let (start, end) = match unit {
            Unit::Word => layer.word_range(caret),
            Unit::Line => layer.line_range(caret),
            Unit::Char => (caret, caret),
        };
        self.anchor = Some((page, start));
        self.unit = unit;
        self.ranges.clear();
        if start != end {
            self.ranges.insert(page, (start, end));
        }
    }

    pub fn extend<'a>(
        &mut self,
        layer: impl Fn(usize) -> Option<&'a Shared>,
        page: usize,
        caret: usize,
    ) -> bool {
        let Some(anchor) = self.anchor else {
            return false;
        };
        let next = desired(layer, anchor, (page, caret), self.unit);
        if next == self.ranges {
            return false;
        }
        self.ranges = next;
        true
    }

    pub fn all<'a>(&mut self, layers: impl Iterator<Item = (usize, &'a Shared)>) {
        self.ranges = layers
            .filter(|(_, l)| l.has_text())
            .map(|(p, l)| (p, (0, l.len())))
            .collect();
    }

    pub fn text<'a>(&self, layer: impl Fn(usize) -> Option<&'a Shared>) -> String {
        let mut text = String::new();
        for (&page, &(start, end)) in &self.ranges {
            let Some(layer) = layer(page) else { continue };
            let part = layer.text_of(start, end);
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
}

fn desired<'a>(
    layer: impl Fn(usize) -> Option<&'a Shared>,
    anchor: (usize, usize),
    caret: (usize, usize),
    unit: Unit,
) -> Ranges {
    let lo = |l: &Shared, i: usize| match unit {
        Unit::Word => l.word_range(i).0,
        Unit::Line => l.line_range(i).0,
        Unit::Char => i,
    };
    let hi = |l: &Shared, i: usize| match unit {
        Unit::Word => l.word_range(i).1,
        Unit::Line => l.line_range(i).1,
        Unit::Char => i,
    };
    let (first, last, first_at, last_at) = if anchor.0 <= caret.0 {
        (anchor.0, caret.0, anchor.1, caret.1)
    } else {
        (caret.0, anchor.0, caret.1, anchor.1)
    };
    let mut out = Ranges::new();
    for page in first..=last {
        let Some(l) = layer(page) else { continue };
        let len = l.len();
        let (start, end) = match (page == first, page == last) {
            (true, true) => (lo(l, first_at.min(last_at)), hi(l, first_at.max(last_at))),
            (true, false) => (lo(l, first_at), len),
            (false, true) => (0, hi(l, last_at)),
            _ => (0, len),
        };
        let (start, end) = (start.min(end), end.min(len));
        if start != end {
            out.insert(page, (start, end));
        }
    }
    out
}
