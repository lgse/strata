//! The name under a tile: wrapped over at most N lines (anywhere inside a word too long for
//! the line, as Strata's `WordChar` wrap), centred, and cut in the middle with an ellipsis
//! when it would take more, so the end that tells names apart (`…final-v3.tar.gz`) stays.
//!
//! Rune can't do this itself at the pinned rev: `max_lines` cuts at the end, and
//! `truncate_middle` makes the text one line (Rune draft U-C3-1, in the module docs). So the
//! cut is worked out here by measuring candidates with `rune::text::measure_with`, the same
//! layout the text element draws, and the shown text is the cut string.
//!
//! Cuts fall between grapheme clusters, as Rune's own do: a letter keeps its combining marks,
//! and an emoji sequence stays whole.

use std::cell::RefCell;
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

/// Measured cuts, keyed by name, font, width and lines, in two generations: a lookup finds
/// either, and a full current generation becomes the old one. So it holds at most twice
/// [`GENERATION`] cuts, and the ones still in use stay.
#[derive(Debug, Default)]
pub struct CaptionCache {
    current: RefCell<HashMap<u64, Cut>>,
    old: RefCell<HashMap<u64, Cut>>,
}

impl CaptionCache {
    /// `name` as a caption `width` points wide shows it in `font`, over at most `lines`.
    pub fn get(&self, name: &str, font: TextStyle, width: f32, lines: usize) -> Cut {
        let key = key(name, font, width, lines);
        if let Some(cut) = self.current.borrow().get(&key) {
            return cut.clone();
        }
        let cut = self
            .old
            .borrow_mut()
            .remove(&key)
            .unwrap_or_else(|| cut(name, font, width, lines));
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
pub fn cut(name: &str, font: TextStyle, width: f32, lines: usize) -> Cut {
    let whole = measure(name, font, width);
    let limit = limit(font, lines.max(1));
    if whole.1 <= limit || name.is_empty() {
        return Cut {
            text: name.into(),
            cut: false,
            size: whole,
        };
    }
    let graphemes: Vec<&str> = name.graphemes(true).collect();
    let candidate = |keep: usize| -> String {
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let mut s: String = graphemes[..head].concat();
        s.push('…');
        s.push_str(&graphemes[graphemes.len() - tail..].concat());
        s
    };
    // The most grapheme clusters kept that still fit: fitting shrinks as more are kept.
    let (mut lo, mut hi) = (0usize, graphemes.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&candidate(mid), font, width).1 <= limit {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let text = candidate(lo);
    let size = measure(&text, font, width);
    Cut {
        text: text.into(),
        cut: true,
        size,
    }
}
