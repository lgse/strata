// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::TextStyle;
use rune::text::RichLayoutOptions;
use unicode_segmentation::UnicodeSegmentation;

/// How many cuts each of the cache's two generations holds: about a few screens of tiles.
const GENERATION: usize = 4096;

/// A name as a caption shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// What the caption draws: the whole name, or its ends around an ellipsis.
    pub text: Rc<str>,
    /// The name was cut.
    pub cut: bool,
    /// The drawn text's size in points: the widest line, and the lines' height.
    pub size: (f32, f32),
}

#[derive(Debug, Default)]
pub struct CaptionCache {
    current: RefCell<HashMap<u64, Cut>>,
    old: RefCell<HashMap<u64, Cut>>,
    /// Last cut length for `(layout_key, keep)` to seed binary search near the answer.
    hint: Cell<(u64, usize)>,
}

impl CaptionCache {
    /// `name` as a caption `width` points wide shows it in `font`, over at most `lines`.
    pub fn get(&self, name: &str, font: TextStyle, width: f32, lines: usize) -> Cut {
        let key = key(name, font, width, lines);
        if let Some(cut) = self.current.borrow().get(&key) {
            return cut.clone();
        }
        let cut = self.old.borrow_mut().remove(&key).unwrap_or_else(|| {
            let layout = CaptionLayout { font, width, lines };
            let lkey = layout.key();
            let (hkey, hkeep) = self.hint.get();
            let seed = if hkey == lkey { hkeep } else { 0 };
            let (cut, keep) = cut_with_hint(name, layout, seed);
            if keep > 0 {
                self.hint.set((lkey, keep));
            }
            cut
        });
        let mut current = self.current.borrow_mut();
        if current.len() >= GENERATION {
            *self.old.borrow_mut() = std::mem::take(&mut *current);
        }
        current.insert(key, cut.clone());
        cut
    }

    /// How many cuts it holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.current.borrow().len() + self.old.borrow().len()
    }
}

#[derive(Clone, Copy)]
struct CaptionLayout {
    font: TextStyle,
    width: f32,
    lines: usize,
}

impl CaptionLayout {
    fn key(self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        self.font.hash(&mut h);
        self.width.to_bits().hash(&mut h);
        self.lines.hash(&mut h);
        rune::text::font_epoch().hash(&mut h);
        h.finish()
    }
}

fn key(name: &str, font: TextStyle, width: f32, lines: usize) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    name.hash(&mut h);
    font.hash(&mut h);
    width.to_bits().hash(&mut h);
    lines.hash(&mut h);
    // Fonts loaded since (a text size or family change) measure otherwise.
    rune::text::font_epoch().hash(&mut h);
    h.finish()
}

fn measure(text: &str, font: TextStyle, width: f32) -> (f32, f32) {
    rune::text::measure_with(
        text,
        &[],
        font,
        Some(width),
        RichLayoutOptions::new().wrap_anywhere(),
    )
}

/// The tallest `lines` lines may be, with half a point for rounding.
fn limit(font: TextStyle, lines: usize) -> f32 {
    lines as f32 * strata_kit::typography::line_of(font) + 0.5
}

/// `name` cut to fit `lines` lines `width` wide: the longest ends around "…" that fit.
#[cfg(test)]
pub fn cut(name: &str, font: TextStyle, width: f32, lines: usize) -> Cut {
    cut_with_hint(name, CaptionLayout { font, width, lines }, 0).0
}

fn cut_with_hint(name: &str, layout: CaptionLayout, hint: usize) -> (Cut, usize) {
    let whole = measure(name, layout.font, layout.width);
    let limit = limit(layout.font, layout.lines.max(1));
    if whole.1 <= limit || name.is_empty() {
        return (
            Cut {
                text: name.into(),
                cut: false,
                size: whole,
            },
            0,
        );
    }
    let mut search = CutSearch::new(name, layout.font, layout.width, limit);
    search.narrow_from_hint(hint);
    search.finish()
}

struct CutSearch<'a> {
    name: &'a str,
    starts: Vec<usize>,
    buf: String,
    font: TextStyle,
    width: f32,
    limit: f32,
    lo: usize,
    hi: usize,
    best_size: Option<(f32, f32)>,
}

impl<'a> CutSearch<'a> {
    fn new(name: &'a str, font: TextStyle, width: f32, limit: f32) -> Self {
        let starts: Vec<usize> = name.grapheme_indices(true).map(|(i, _)| i).collect();
        let hi = starts.len().saturating_sub(1);
        Self {
            name,
            starts,
            buf: String::with_capacity(name.len() + '…'.len_utf8()),
            font,
            width,
            limit,
            lo: 0,
            hi,
            best_size: None,
        }
    }

    fn fill(&mut self, keep: usize) {
        self.buf.clear();
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        if head > 0 {
            let head_end = self.starts.get(head).copied().unwrap_or(self.name.len());
            self.buf.push_str(&self.name[..head_end]);
        }
        self.buf.push('…');
        if tail > 0 {
            let tail_start = self.starts[self.starts.len() - tail];
            self.buf.push_str(&self.name[tail_start..]);
        }
    }

    fn probe(&mut self, keep: usize) -> Option<(f32, f32)> {
        self.fill(keep);
        let size = measure(&self.buf, self.font, self.width);
        (size.1 <= self.limit).then_some(size)
    }

    fn narrow_from_hint(&mut self, hint: usize) {
        if hint == 0 || hint > self.hi {
            return;
        }
        if let Some(size) = self.probe(hint) {
            self.lo = hint;
            self.best_size = Some(size);
            if hint < self.hi {
                match self.probe(hint + 1) {
                    Some(next) => {
                        self.lo = hint + 1;
                        self.best_size = Some(next);
                    }
                    None => self.hi = hint,
                }
            }
            return;
        }
        self.hi = hint - 1;
        if self.hi > 0 {
            match self.probe(self.hi) {
                Some(prev) => {
                    self.lo = self.hi;
                    self.best_size = Some(prev);
                }
                None => self.hi -= 1,
            }
        }
    }

    fn finish(mut self) -> (Cut, usize) {
        // The most grapheme clusters kept that still fit: fitting shrinks as more are kept.
        while self.lo < self.hi {
            let mid = (self.lo + self.hi).div_ceil(2);
            if let Some(size) = self.probe(mid) {
                self.lo = mid;
                self.best_size = Some(size);
            } else {
                self.hi = mid - 1;
            }
        }
        self.fill(self.lo);
        let size = self
            .best_size
            .unwrap_or_else(|| measure(&self.buf, self.font, self.width));
        let keep = self.lo;
        (
            Cut {
                text: self.buf.into(),
                cut: true,
                size,
            },
            keep,
        )
    }
}
