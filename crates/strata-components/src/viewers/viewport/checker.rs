//! The checkerboard drawn behind content that may be transparent.
//!
//! Rune has no repeating pattern fill, so the board is a `canvas` shape: one square segment
//! per dark square over a light fill. Its squares are `look.space.md` points, in screen space,
//! so they keep their size at every zoom, and the board is never larger than the frame, which
//! bounds the squares (about 4,800 in a 960 × 640 frame). It is memoized on its size and
//! colours, so a commit that doesn't change them reuses it.
//!
//! The colours are two Kit surface roles: `surface_raised` (the light squares and the fill) and
//! `surface_base` (the dark squares).

use rune::{Color, El, LineCap, Shape, canvas, memo};
use strata_kit::look::Look;

/// A checkerboard `size` points across, keyed `key`, for drawing behind transparent content.
/// Its corner square is light.
///
/// C12 puts one behind a transparent page; the viewport puts one behind its content, the size
/// of the content's visible part.
pub fn checkerboard(key: impl Into<String>, size: [f32; 2], look: &Look) -> El {
    let square = look.space.md.max(1.0);
    let light = look.kit.c.surface_raised;
    let dark = look.kit.c.surface_base;
    let version = (
        size[0].to_bits(),
        size[1].to_bits(),
        square.to_bits(),
        light,
        dark,
    );
    memo(key, version, move || board(size, square, light, dark))
}

fn board(size: [f32; 2], square: f32, light: Color, dark: Color) -> El {
    let (cols, rows) = (
        (size[0] / square).ceil().max(0.0) as usize,
        (size[1] / square).ceil().max(0.0) as usize,
    );
    let mut shape = Shape::new();
    for r in 0..rows {
        let top = r as f32 * square;
        // The last row and column stop at the board's edge.
        let height = square.min(size[1] - top);
        let y = top + height / 2.0;
        // Dark squares are where row + column is odd.
        for c in ((r + 1) % 2..cols).step_by(2) {
            let x = c as f32 * square;
            let end = (x + square).min(size[0]);
            shape = shape.line([x, y], [end, y], height, LineCap::Butt, dark);
        }
    }
    canvas(shape)
        .size(size[0], size[1])
        .fill(light)
        .a11y_hidden()
        .inert()
}
