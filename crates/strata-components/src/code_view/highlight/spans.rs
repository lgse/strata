//! Classes to colour-only Rune spans.

use std::ops::Range;

use rune::{Color, Span};

use super::engine::Token;

/// Rune numbers a text element's spans with `Ink(i as u16 + 1)` (`rune/src/text.rs`, where
/// `Ink(0)` is the element's own colour), so the 65,536th span overflows: a panic in a debug
/// build, and in a release one it and every later span take another span's colour. No more
/// than this many go to one element.
pub const MAX_SPANS: usize = u16::MAX as usize;

/// `tokens`' coloured runs, moved `offset` bytes on, appended to `out`. Tokens in the default
/// colour are left out, and touching runs of one colour join.
pub(crate) fn colored(
    tokens: &[Token],
    palette: &[Option<Color>],
    offset: usize,
    out: &mut Vec<(Range<usize>, Color)>,
) {
    for &(a, b, class) in tokens {
        let Some(Some(color)) = palette.get(class as usize) else {
            continue;
        };
        let range = a as usize + offset..b as usize + offset;
        if let Some(last) = out.last_mut()
            && last.0.end == range.start
            && last.1 == *color
        {
            last.0.end = range.end;
            continue;
        }
        out.push((range, *color));
    }
}

/// A colour-only span: highlighting never changes weight or style, so nothing moves.
pub(crate) fn span(range: Range<usize>, color: Color) -> Span {
    Span::new(range.start, range.end).color(color)
}
