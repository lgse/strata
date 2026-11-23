// SPDX-License-Identifier: MIT

//! Classes to colour-only Rune spans.

use std::ops::Range;

use rune::{Color, Span};

use super::engine::Token;

pub const MAX_SPANS: usize = u16::MAX as usize;

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
